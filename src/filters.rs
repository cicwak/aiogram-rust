//! Common dispatcher filters and a closure-based extension point.

use std::fmt;
use std::future::Future;
use std::ops::{Add, BitAnd, BitOr, BitXor, Div, Mul, Neg, Not as StdNot, Rem, Shl, Shr, Sub};
use std::pin::Pin;
use std::sync::Arc;

use crate::dispatcher::{HandlerFlags, UpdateContext};
use crate::enums::ContentType;
use crate::fsm::{FsmContext, State, StatesGroup};
use crate::types::{ChatMemberUnion, ChatMemberUpdated};

pub type FilterFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

pub trait Filter: Send + Sync {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a>;

    fn update_handler_flags(&self, _flags: &mut HandlerFlags) {}
}

pub trait FilterExt: Filter + Sized + 'static {
    fn and<F: Filter + 'static>(self, other: F) -> And<Self, F> {
        And(self, other)
    }

    fn or<F: Filter + 'static>(self, other: F) -> Or<Self, F> {
        Or(self, other)
    }

    fn not(self) -> Not<Self> {
        Not(self)
    }
}

impl<T: Filter + Sized + 'static> FilterExt for T {}

pub struct And<A, B>(A, B);

impl<A: Filter, B: Filter> Filter for And<A, B> {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move { self.0.check(context).await && self.1.check(context).await })
    }
}

pub struct Or<A, B>(A, B);

impl<A: Filter, B: Filter> Filter for Or<A, B> {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move { self.0.check(context).await || self.1.check(context).await })
    }
}

pub struct Not<F>(F);

impl<F: Filter> Filter for Not<F> {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move { !self.0.check(context).await })
    }
}

pub fn all(filters: Vec<Arc<dyn Filter>>) -> impl Filter {
    FnFilter::new(move |context| {
        let filters = filters.clone();
        Box::pin(async move {
            for filter in filters.iter() {
                if !filter.check(context).await {
                    return false;
                }
            }
            true
        })
    })
}

pub fn either(filters: Vec<Arc<dyn Filter>>) -> impl Filter {
    FnFilter::new(move |context| {
        let filters = filters.clone();
        Box::pin(async move {
            for filter in filters.iter() {
                if filter.check(context).await {
                    return true;
                }
            }
            false
        })
    })
}

pub fn boxed(filter: impl Filter + 'static) -> Arc<dyn Filter> {
    Arc::new(filter)
}

#[derive(Clone)]
pub struct FnFilter(Arc<dyn for<'a> Fn(&'a UpdateContext) -> FilterFuture<'a> + Send + Sync>);

impl FnFilter {
    pub fn new<F>(filter: F) -> Self
    where
        F: for<'a> Fn(&'a UpdateContext) -> FilterFuture<'a> + Send + Sync + 'static,
    {
        Self(Arc::new(filter))
    }
}

impl Filter for FnFilter {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        (self.0)(context)
    }
}

pub fn any() -> impl Filter {
    FnFilter::new(|_| Box::pin(async { true }))
}

pub fn text(expected: impl Into<String>) -> impl Filter {
    let expected = Arc::new(expected.into());
    FnFilter::new(move |context| {
        let expected = expected.clone();
        Box::pin(async move {
            context
                .message()
                .and_then(|message| message.text.as_deref())
                == Some(expected.as_str())
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandObject<'a> {
    pub prefix: char,
    pub command: &'a str,
    pub mention: Option<&'a str>,
    pub args: Option<&'a str>,
}

/// Owned command details injected into a matching handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandMatch {
    pub prefix: char,
    pub command: String,
    pub mention: Option<String>,
    pub args: Option<String>,
    pub regex_captures: Option<Vec<Option<String>>>,
}

impl From<CommandObject<'_>> for CommandMatch {
    fn from(value: CommandObject<'_>) -> Self {
        Self {
            prefix: value.prefix,
            command: value.command.to_owned(),
            mention: value.mention.map(str::to_owned),
            args: value.args.map(str::to_owned),
            regex_captures: None,
        }
    }
}

pub fn parse_command(text: &str) -> Option<CommandObject<'_>> {
    parse_command_with_prefix(text, &['/'])
}

fn parse_command_with_prefix<'a>(text: &'a str, prefixes: &[char]) -> Option<CommandObject<'a>> {
    let prefix = text.chars().next()?;
    if !prefixes.contains(&prefix) {
        return None;
    }
    let tail = &text[prefix.len_utf8()..];
    let split = tail.find(char::is_whitespace).unwrap_or(tail.len());
    let head = &tail[..split];
    if head.is_empty() {
        return None;
    }
    let args = tail[split..].trim_start();
    let (command, mention) = match head.split_once('@') {
        Some((command, mention)) if !command.is_empty() && !mention.is_empty() => {
            (command, Some(mention))
        }
        Some(_) => return None,
        None => (head, None),
    };
    Some(CommandObject {
        prefix,
        command,
        mention,
        args: (!args.is_empty()).then_some(args),
    })
}

/// Configurable command filter with aiogram-style prefixes, mentions, and case handling.
#[derive(Debug, Clone)]
pub struct Command {
    names: Vec<String>,
    patterns: Vec<regex::Regex>,
    prefixes: Vec<char>,
    ignore_case: bool,
    ignore_mention: bool,
    bot_username: Option<String>,
}

impl Command {
    pub fn new(name: impl Into<String>) -> Self {
        Self::many([name])
    }

    pub fn many(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            names: names
                .into_iter()
                .map(|name| name.into().trim_start_matches('/').to_owned())
                .collect(),
            patterns: Vec::new(),
            prefixes: vec!['/'],
            ignore_case: false,
            ignore_mention: false,
            bot_username: None,
        }
    }

    pub fn regex(pattern: &str) -> crate::Result<Self> {
        Ok(Self {
            names: Vec::new(),
            patterns: vec![
                regex::Regex::new(pattern)
                    .map_err(|error| crate::Error::Utility(format!("invalid regex: {error}")))?,
            ],
            prefixes: vec!['/'],
            ignore_case: false,
            ignore_mention: false,
            bot_username: None,
        })
    }

    pub fn add_regex(mut self, pattern: &str) -> crate::Result<Self> {
        self.patterns.push(
            regex::Regex::new(pattern)
                .map_err(|error| crate::Error::Utility(format!("invalid regex: {error}")))?,
        );
        Ok(self)
    }

    pub fn prefixes(mut self, prefixes: impl IntoIterator<Item = char>) -> Self {
        self.prefixes = prefixes.into_iter().collect();
        self
    }

    pub fn ignore_case(mut self, value: bool) -> Self {
        self.ignore_case = value;
        self
    }

    pub fn ignore_mention(mut self, value: bool) -> Self {
        self.ignore_mention = value;
        self
    }

    pub fn bot_username(mut self, value: impl Into<String>) -> Self {
        self.bot_username = Some(value.into().trim_start_matches('@').to_owned());
        self
    }
}

impl Filter for Command {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            let Some(text) = context
                .message()
                .and_then(|message| message.text.as_deref().or(message.caption.as_deref()))
            else {
                return false;
            };
            let Some(parsed) = parse_command_with_prefix(text, &self.prefixes) else {
                return false;
            };
            let name_matches = self.names.iter().any(|name| {
                if self.ignore_case {
                    caseless::default_case_fold_str(parsed.command)
                        == caseless::default_case_fold_str(name)
                } else {
                    parsed.command == name
                }
            });
            let regex_captures = self.patterns.iter().find_map(|pattern| {
                pattern
                    .captures(parsed.command)
                    .filter(|captures| captures.get(0).is_some_and(|capture| capture.start() == 0))
                    .map(|captures| {
                        captures
                            .iter()
                            .map(|capture| capture.map(|capture| capture.as_str().to_owned()))
                            .collect::<Vec<_>>()
                    })
            });
            if !name_matches && regex_captures.is_none() {
                return false;
            }
            let mention_matches = match parsed.mention {
                None => true,
                Some(_) if self.ignore_mention => true,
                Some(actual) => {
                    let expected = match self.bot_username.as_deref() {
                        Some(expected) => Some(expected.to_owned()),
                        None => context.bot.get_me().await.ok().and_then(|bot| bot.username),
                    };
                    expected.is_some_and(|expected| actual.eq_ignore_ascii_case(&expected))
                }
            };
            if mention_matches {
                let mut command = CommandMatch::from(parsed);
                command.regex_captures = regex_captures;
                context.inject_dependency(command);
            }
            mention_matches
        })
    }

    fn update_handler_flags(&self, flags: &mut HandlerFlags) {
        let mut commands = flags
            .get_cloned::<Vec<Command>>("commands")
            .unwrap_or_default();
        commands.push(self.clone());
        flags.insert("commands", commands);
    }
}

/// Matches `/command`, `/command@bot_name`, and an optional argument tail.
pub fn command(name: impl Into<String>) -> Command {
    Command::new(name)
}

/// `/start` command filter with optional deep-link presence and Base64URL decoding.
#[derive(Debug, Clone)]
pub struct CommandStart {
    command: Command,
    deep_link: Option<bool>,
    deep_link_encoded: bool,
}

impl Default for CommandStart {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandStart {
    pub fn new() -> Self {
        Self {
            command: Command::new("start"),
            deep_link: None,
            deep_link_encoded: false,
        }
    }

    pub fn deep_link(mut self, required: bool) -> Self {
        self.deep_link = Some(required);
        self
    }

    pub fn encoded(mut self, value: bool) -> Self {
        self.deep_link_encoded = value;
        self
    }

    pub fn ignore_case(mut self, value: bool) -> Self {
        self.command = self.command.ignore_case(value);
        self
    }

    pub fn ignore_mention(mut self, value: bool) -> Self {
        self.command = self.command.ignore_mention(value);
        self
    }
}

impl Filter for CommandStart {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            if !self.command.check(context).await {
                return false;
            }
            let Some(command) = context.dependency::<CommandMatch>() else {
                return false;
            };
            match self.deep_link {
                Some(true) if command.args.is_none() => return false,
                Some(false) if command.args.is_some() => return false,
                _ => {}
            }
            if self.deep_link_encoded
                && let Some(payload) = command.args.as_deref()
            {
                let Ok(decoded) = crate::utils::deep_linking::decode_payload(payload) else {
                    return false;
                };
                let Ok(decoded) = String::from_utf8(decoded) else {
                    return false;
                };
                let mut command = command.as_ref().clone();
                command.args = Some(decoded);
                context.inject_dependency(command);
            }
            true
        })
    }

    fn update_handler_flags(&self, flags: &mut HandlerFlags) {
        self.command.update_handler_flags(flags);
    }
}

pub fn content_type(expected: ContentType) -> impl Filter {
    FnFilter::new(move |context| {
        Box::pin(async move {
            context
                .message()
                .is_some_and(|message| message.content_type() == expected)
        })
    })
}

#[derive(Debug, Clone)]
enum StateMatcher {
    Any,
    None,
    Exact(State),
    Group(StatesGroup),
}

/// Matches the state injected by `FsmMiddleware`.
#[derive(Debug, Clone)]
pub struct StateFilter {
    matchers: Vec<StateMatcher>,
}

impl StateFilter {
    pub fn new(state: State) -> Self {
        Self {
            matchers: vec![StateMatcher::Exact(state)],
        }
    }

    pub fn any() -> Self {
        Self {
            matchers: vec![StateMatcher::Any],
        }
    }

    pub fn none() -> Self {
        Self {
            matchers: vec![StateMatcher::None],
        }
    }

    pub fn group(group: StatesGroup) -> Self {
        Self {
            matchers: vec![StateMatcher::Group(group)],
        }
    }

    pub fn or_state(mut self, state: State) -> Self {
        self.matchers.push(StateMatcher::Exact(state));
        self
    }
}

impl Filter for StateFilter {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            let Some(fsm) = context.dependency::<FsmContext>() else {
                return false;
            };
            let Ok(raw) = fsm.get_state().await else {
                return false;
            };
            self.matchers.iter().any(|matcher| match matcher {
                StateMatcher::Any => true,
                StateMatcher::None => raw.is_none(),
                StateMatcher::Exact(state) => state.matches(raw.as_deref()),
                StateMatcher::Group(group) => raw.as_deref().is_some_and(|raw| group.contains(raw)),
            })
        })
    }
}

pub fn callback_data(expected: impl Into<String>) -> impl Filter {
    let expected = Arc::new(expected.into());
    FnFilter::new(move |context| {
        let expected = expected.clone();
        Box::pin(async move {
            context
                .callback_query()
                .and_then(|query| query.data.as_deref())
                == Some(expected.as_str())
        })
    })
}

pub fn callback_prefix(expected: impl Into<String>) -> impl Filter {
    let expected = Arc::new(format!("{}:", expected.into().trim_end_matches(':')));
    FnFilter::new(move |context| {
        let expected = expected.clone();
        Box::pin(async move {
            context
                .callback_query()
                .and_then(|query| query.data.as_deref())
                .is_some_and(|data| data.starts_with(expected.as_str()))
        })
    })
}

pub fn callback_data_filter<T, Parse, Predicate>(parse: Parse, predicate: Predicate) -> impl Filter
where
    T: Send + Sync + 'static,
    Parse: Fn(&str) -> crate::Result<T> + Send + Sync + 'static,
    Predicate: Fn(&T) -> bool + Send + Sync + 'static,
{
    let parse = Arc::new(parse);
    let predicate = Arc::new(predicate);
    FnFilter::new(move |context| {
        let parse = parse.clone();
        let predicate = predicate.clone();
        Box::pin(async move {
            let Some(data) = context
                .callback_query()
                .and_then(|query| query.data.as_deref())
            else {
                return false;
            };
            let Ok(value) = parse(data) else {
                return false;
            };
            if !predicate(&value) {
                return false;
            }
            context.inject_dependency(value);
            true
        })
    })
}

/// Matches typed dependency-injection data, corresponding to aiogram's
/// `MagicData` capability without giving up Rust's static type checks.
pub fn dependency<T, Predicate>(predicate: Predicate) -> impl Filter
where
    T: Send + Sync + 'static,
    Predicate: Fn(&T) -> bool + Send + Sync + 'static,
{
    let predicate = Arc::new(predicate);
    FnFilter::new(move |context| {
        let predicate = predicate.clone();
        Box::pin(async move {
            context
                .dependency::<T>()
                .is_some_and(|value| predicate(&value))
        })
    })
}

/// Filters error observer events with a typed predicate.
pub fn error<Predicate>(predicate: Predicate) -> impl Filter
where
    Predicate: Fn(&crate::Error) -> bool + Send + Sync + 'static,
{
    let predicate = Arc::new(predicate);
    FnFilter::new(move |context| {
        let predicate = predicate.clone();
        Box::pin(async move { context.error().is_some_and(|error| predicate(error)) })
    })
}

pub fn error_message_contains(expected: impl Into<String>) -> impl Filter {
    let expected = expected.into();
    error(move |error| error.to_string().contains(&expected))
}

#[derive(Debug, Clone, Copy)]
enum MagicRoot {
    Event,
    Update,
}

/// A dynamically addressed Telegram field with aiogram `F`-style predicates.
/// Paths use serialized field names such as `text`, `chat.id`, or
/// `message.text` when created through [`update_field`].
#[derive(Clone)]
pub struct MagicField {
    root: MagicRoot,
    path: Arc<Vec<String>>,
    transforms: Arc<Vec<MagicTransform>>,
}

#[derive(Clone)]
enum MagicTransform {
    Lowercase,
    Casefold,
    Uppercase,
    Trim,
    Length,
    Add(f64),
    Subtract(f64),
    Multiply(f64),
    Divide(f64),
    FloorDivide(f64),
    Modulo(f64),
    Power(f64),
    BitAnd(i64),
    BitOr(i64),
    BitXor(i64),
    ShiftLeft(u32),
    ShiftRight(u32),
    Negate,
    Custom(Arc<dyn Fn(serde_json::Value) -> Option<serde_json::Value> + Send + Sync>),
}

impl fmt::Debug for MagicField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MagicField")
            .field("root", &self.root)
            .field("path", &self.path)
            .field("transform_count", &self.transforms.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegexMode {
    Match,
    Search,
    FullMatch,
    FindAll,
    FindIter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicRegexMatch {
    pub matched: String,
    pub start: usize,
    pub end: usize,
    pub captures: Vec<Option<String>>,
    pub named: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicRegexMatches {
    pub matches: Vec<MagicRegexMatch>,
}

impl MagicRegexMatches {
    pub fn first(&self) -> Option<&MagicRegexMatch> {
        self.matches.first()
    }

    pub fn len(&self) -> usize {
        self.matches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }
}

/// Selects a field relative to the current Telegram event (`F.text` in aiogram).
pub fn field(path: impl AsRef<str>) -> MagicField {
    MagicField::new(MagicRoot::Event, path.as_ref())
}

/// Selects a field relative to the complete update.
pub fn update_field(path: impl AsRef<str>) -> MagicField {
    MagicField::new(MagicRoot::Update, path.as_ref())
}

impl MagicField {
    fn new(root: MagicRoot, path: &str) -> Self {
        Self {
            root,
            path: Arc::new(
                path.split('.')
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            transforms: Arc::new(Vec::new()),
        }
    }

    fn with_transform(mut self, transform: MagicTransform) -> Self {
        Arc::make_mut(&mut self.transforms).push(transform);
        self
    }

    fn resolve(&self, context: &UpdateContext) -> Option<serde_json::Value> {
        let update = serde_json::to_value(context.update.as_ref()).ok()?;
        let mut value = match self.root {
            MagicRoot::Update => &update,
            MagicRoot::Event => update.get(context.update.event_type()?)?,
        };
        for segment in self.path.iter() {
            value = match value {
                serde_json::Value::Object(object) => object.get(segment)?,
                serde_json::Value::Array(array) => array.get(segment.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        let mut value = value.clone();
        for transform in self.transforms.iter() {
            value = apply_magic_transform(transform, value)?;
        }
        Some(value)
    }

    fn predicate(
        self,
        predicate: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
    ) -> FnFilter {
        let predicate = Arc::new(predicate);
        FnFilter::new(move |context| {
            let value = self.resolve(context);
            let predicate = predicate.clone();
            Box::pin(async move { value.as_ref().is_some_and(|value| predicate(value)) })
        })
    }

    pub fn exists(self) -> FnFilter {
        self.predicate(json_truthy)
    }

    pub fn is_null(self) -> FnFilter {
        self.predicate(serde_json::Value::is_null)
    }

    pub fn is_not_null(self) -> FnFilter {
        self.predicate(|value| !value.is_null())
    }

    pub fn equals(self, expected: impl serde::Serialize) -> FnFilter {
        let expected = serde_json::to_value(expected).unwrap_or(serde_json::Value::Null);
        self.predicate(move |value| json_equal(value, &expected))
    }

    pub fn not_equals(self, expected: impl serde::Serialize) -> FnFilter {
        let expected = serde_json::to_value(expected).unwrap_or(serde_json::Value::Null);
        self.predicate(move |value| !json_equal(value, &expected))
    }

    pub fn contains(self, expected: impl serde::Serialize) -> FnFilter {
        let expected = serde_json::to_value(expected).unwrap_or(serde_json::Value::Null);
        self.predicate(move |value| match value {
            serde_json::Value::String(value) => expected
                .as_str()
                .is_some_and(|expected| value.contains(expected)),
            serde_json::Value::Array(values) => values.contains(&expected),
            _ => false,
        })
    }

    pub fn not_contains(self, expected: impl serde::Serialize) -> FnFilter {
        self.contains(expected).not_filter()
    }

    pub fn starts_with(self, expected: impl Into<String>) -> FnFilter {
        let expected = expected.into();
        self.predicate(move |value| {
            value
                .as_str()
                .is_some_and(|value| value.starts_with(&expected))
        })
    }

    pub fn ends_with(self, expected: impl Into<String>) -> FnFilter {
        let expected = expected.into();
        self.predicate(move |value| {
            value
                .as_str()
                .is_some_and(|value| value.ends_with(&expected))
        })
    }

    pub fn greater_than(self, expected: f64) -> FnFilter {
        self.predicate(move |value| value.as_f64().is_some_and(|value| value > expected))
    }

    pub fn less_than(self, expected: f64) -> FnFilter {
        self.predicate(move |value| value.as_f64().is_some_and(|value| value < expected))
    }

    pub fn greater_or_equal(self, expected: f64) -> FnFilter {
        self.predicate(move |value| value.as_f64().is_some_and(|value| value >= expected))
    }

    pub fn less_or_equal(self, expected: f64) -> FnFilter {
        self.predicate(move |value| value.as_f64().is_some_and(|value| value <= expected))
    }

    pub fn one_of(self, expected: impl IntoIterator<Item = impl serde::Serialize>) -> FnFilter {
        let expected: Vec<_> = expected
            .into_iter()
            .filter_map(|value| serde_json::to_value(value).ok())
            .collect();
        self.predicate(move |value| expected.iter().any(|expected| json_equal(value, expected)))
    }

    pub fn not_one_of(self, expected: impl IntoIterator<Item = impl serde::Serialize>) -> FnFilter {
        self.one_of(expected).not_filter()
    }

    pub fn regex(self, pattern: &str) -> crate::Result<FnFilter> {
        self.regex_with_mode(pattern, RegexMode::Match)
    }

    pub fn regex_search(self, pattern: &str) -> crate::Result<FnFilter> {
        self.regex_with_mode(pattern, RegexMode::Search)
    }

    pub fn regex_full_match(self, pattern: &str) -> crate::Result<FnFilter> {
        self.regex_with_mode(pattern, RegexMode::FullMatch)
    }

    pub fn regex_find_all(self, pattern: &str) -> crate::Result<FnFilter> {
        self.regex_with_mode(pattern, RegexMode::FindAll)
    }

    pub fn regex_find_iter(self, pattern: &str) -> crate::Result<FnFilter> {
        self.regex_with_mode(pattern, RegexMode::FindIter)
    }

    pub fn regex_with_mode(self, pattern: &str, mode: RegexMode) -> crate::Result<FnFilter> {
        let regex = regex::Regex::new(pattern)
            .map_err(|error| crate::Error::Utility(format!("invalid regex: {error}")))?;
        Ok(FnFilter::new(move |context| {
            let value = self.resolve(context);
            let regex = regex.clone();
            Box::pin(async move {
                let Some(value) = value.as_ref().and_then(serde_json::Value::as_str) else {
                    return false;
                };
                if matches!(mode, RegexMode::FindAll | RegexMode::FindIter) {
                    let matches = regex
                        .captures_iter(value)
                        .filter_map(|captures| {
                            let found = captures.get(0)?;
                            let named = regex
                                .capture_names()
                                .flatten()
                                .filter_map(|name| {
                                    captures
                                        .name(name)
                                        .map(|value| (name.to_owned(), value.as_str().to_owned()))
                                })
                                .collect();
                            Some(MagicRegexMatch {
                                matched: found.as_str().to_owned(),
                                start: found.start(),
                                end: found.end(),
                                captures: captures
                                    .iter()
                                    .map(|capture| capture.map(|value| value.as_str().to_owned()))
                                    .collect(),
                                named,
                            })
                        })
                        .collect::<Vec<_>>();
                    if matches.is_empty() {
                        return false;
                    }
                    context.inject_dependency(MagicRegexMatches { matches });
                    return true;
                }
                let Some(captures) = regex.captures(value) else {
                    return false;
                };
                let Some(found) = captures.get(0) else {
                    return false;
                };
                let matches_mode = match mode {
                    RegexMode::Match => found.start() == 0,
                    RegexMode::Search => true,
                    RegexMode::FullMatch => found.start() == 0 && found.end() == value.len(),
                    RegexMode::FindAll | RegexMode::FindIter => unreachable!(),
                };
                if !matches_mode {
                    return false;
                }
                let named = regex
                    .capture_names()
                    .flatten()
                    .filter_map(|name| {
                        captures
                            .name(name)
                            .map(|value| (name.to_owned(), value.as_str().to_owned()))
                    })
                    .collect();
                context.inject_dependency(MagicRegexMatch {
                    matched: found.as_str().to_owned(),
                    start: found.start(),
                    end: found.end(),
                    captures: captures
                        .iter()
                        .map(|capture| capture.map(|value| value.as_str().to_owned()))
                        .collect(),
                    named,
                });
                true
            })
        }))
    }

    pub fn lowercase(self) -> Self {
        self.with_transform(MagicTransform::Lowercase)
    }

    pub fn lower(self) -> Self {
        self.lowercase()
    }

    pub fn upper(self) -> Self {
        self.with_transform(MagicTransform::Uppercase)
    }

    pub fn casefold(self) -> Self {
        self.with_transform(MagicTransform::Casefold)
    }

    pub fn trim(self) -> Self {
        self.with_transform(MagicTransform::Trim)
    }

    pub fn length(self) -> Self {
        self.with_transform(MagicTransform::Length)
    }

    pub fn len(self) -> Self {
        self.length()
    }

    pub fn is_empty(&self) -> bool {
        self.path.is_empty()
    }

    /// Appends an object key, corresponding to magic-filter's `F["key"]`.
    pub fn item(mut self, key: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.path).push(key.into());
        self
    }

    /// Appends an array index, corresponding to magic-filter's `F[index]`.
    pub fn index(self, index: usize) -> Self {
        self.item(index.to_string())
    }

    /// Keeps a resolved value only when a Rust predicate accepts it. This is
    /// the type-safe counterpart of magic-filter's selector operation.
    pub fn select(
        self,
        predicate: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.with_transform(MagicTransform::Custom(Arc::new(move |value| {
            predicate(&value).then_some(value)
        })))
    }

    pub fn any(
        self,
        predicate: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
    ) -> FnFilter {
        self.predicate(move |value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().any(&predicate))
        })
    }

    pub fn all(
        self,
        predicate: impl Fn(&serde_json::Value) -> bool + Send + Sync + 'static,
    ) -> FnFilter {
        self.predicate(move |value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().all(&predicate))
        })
    }

    pub fn plus(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Add(value))
    }

    pub fn subtract(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Subtract(value))
    }

    pub fn multiply(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Multiply(value))
    }

    pub fn divide(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Divide(value))
    }

    pub fn floor_divide(self, value: f64) -> Self {
        self.with_transform(MagicTransform::FloorDivide(value))
    }

    pub fn modulo(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Modulo(value))
    }

    pub fn power(self, value: f64) -> Self {
        self.with_transform(MagicTransform::Power(value))
    }

    pub fn bit_and(self, value: i64) -> Self {
        self.with_transform(MagicTransform::BitAnd(value))
    }

    pub fn bit_or(self, value: i64) -> Self {
        self.with_transform(MagicTransform::BitOr(value))
    }

    pub fn bit_xor(self, value: i64) -> Self {
        self.with_transform(MagicTransform::BitXor(value))
    }

    pub fn shift_left(self, value: u32) -> Self {
        self.with_transform(MagicTransform::ShiftLeft(value))
    }

    pub fn shift_right(self, value: u32) -> Self {
        self.with_transform(MagicTransform::ShiftRight(value))
    }

    pub fn negate(self) -> Self {
        self.with_transform(MagicTransform::Negate)
    }

    pub fn map<T, R>(self, transform: impl Fn(T) -> Option<R> + Send + Sync + 'static) -> Self
    where
        T: serde::de::DeserializeOwned,
        R: serde::Serialize,
    {
        self.with_transform(MagicTransform::Custom(Arc::new(move |value| {
            let value = serde_json::from_value(value).ok()?;
            serde_json::to_value(transform(value)?).ok()
        })))
    }

    /// Aiogram-compatible semantic alias for a custom function operation.
    pub fn func<T, R>(self, transform: impl Fn(T) -> Option<R> + Send + Sync + 'static) -> Self
    where
        T: serde::de::DeserializeOwned,
        R: serde::Serialize,
    {
        self.map(transform)
    }

    /// Rust's deserialization/serialization pair is the equivalent of
    /// magic-filter's dynamic cast operation.
    pub fn cast<T, R>(self, transform: impl Fn(T) -> Option<R> + Send + Sync + 'static) -> Self
    where
        T: serde::de::DeserializeOwned,
        R: serde::Serialize,
    {
        self.map(transform)
    }

    /// Deserializes the selected value and injects it into the matching handler.
    pub fn capture<T>(self) -> FnFilter
    where
        T: serde::de::DeserializeOwned + Send + Sync + 'static,
    {
        FnFilter::new(move |context| {
            let value = self.resolve(context);
            Box::pin(async move {
                let Some(value) = value else {
                    return false;
                };
                let Ok(value) = serde_json::from_value::<T>(value) else {
                    return false;
                };
                context.inject_dependency(value);
                true
            })
        })
    }
}

impl Add<f64> for MagicField {
    type Output = Self;

    fn add(self, rhs: f64) -> Self::Output {
        self.plus(rhs)
    }
}

impl Sub<f64> for MagicField {
    type Output = Self;

    fn sub(self, rhs: f64) -> Self::Output {
        self.subtract(rhs)
    }
}

impl Mul<f64> for MagicField {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        self.multiply(rhs)
    }
}

impl Div<f64> for MagicField {
    type Output = Self;

    fn div(self, rhs: f64) -> Self::Output {
        self.divide(rhs)
    }
}

impl Rem<f64> for MagicField {
    type Output = Self;

    fn rem(self, rhs: f64) -> Self::Output {
        self.modulo(rhs)
    }
}

impl BitAnd<i64> for MagicField {
    type Output = Self;

    fn bitand(self, rhs: i64) -> Self::Output {
        self.bit_and(rhs)
    }
}

impl BitOr<i64> for MagicField {
    type Output = Self;

    fn bitor(self, rhs: i64) -> Self::Output {
        self.bit_or(rhs)
    }
}

impl BitXor<i64> for MagicField {
    type Output = Self;

    fn bitxor(self, rhs: i64) -> Self::Output {
        self.bit_xor(rhs)
    }
}

impl Shl<u32> for MagicField {
    type Output = Self;

    fn shl(self, rhs: u32) -> Self::Output {
        self.shift_left(rhs)
    }
}

impl Shr<u32> for MagicField {
    type Output = Self;

    fn shr(self, rhs: u32) -> Self::Output {
        self.shift_right(rhs)
    }
}

impl Neg for MagicField {
    type Output = Self;

    fn neg(self) -> Self::Output {
        self.negate()
    }
}

impl FnFilter {
    fn not_filter(self) -> FnFilter {
        FnFilter::new(move |context| {
            let filter = self.clone();
            Box::pin(async move { !filter.check(context).await })
        })
    }
}

fn json_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => *value,
        serde_json::Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
    }
}

fn json_equal(left: &serde_json::Value, right: &serde_json::Value) -> bool {
    match (left.as_f64(), right.as_f64()) {
        (Some(left), Some(right)) => left == right,
        _ => left == right,
    }
}

fn numeric_value(value: f64) -> Option<serde_json::Value> {
    serde_json::Number::from_f64(value).map(serde_json::Value::Number)
}

fn apply_magic_transform(
    transform: &MagicTransform,
    value: serde_json::Value,
) -> Option<serde_json::Value> {
    match transform {
        MagicTransform::Lowercase => {
            Some(serde_json::Value::String(value.as_str()?.to_lowercase()))
        }
        MagicTransform::Casefold => Some(serde_json::Value::String(
            caseless::default_case_fold_str(value.as_str()?),
        )),
        MagicTransform::Uppercase => {
            Some(serde_json::Value::String(value.as_str()?.to_uppercase()))
        }
        MagicTransform::Trim => Some(serde_json::Value::String(value.as_str()?.trim().to_owned())),
        MagicTransform::Length => {
            let length = match &value {
                serde_json::Value::String(value) => value.chars().count(),
                serde_json::Value::Array(value) => value.len(),
                serde_json::Value::Object(value) => value.len(),
                _ => return None,
            };
            Some(serde_json::Value::from(length))
        }
        MagicTransform::Add(right) => numeric_value(value.as_f64()? + right),
        MagicTransform::Subtract(right) => numeric_value(value.as_f64()? - right),
        MagicTransform::Multiply(right) => numeric_value(value.as_f64()? * right),
        MagicTransform::Divide(right) => (*right != 0.0)
            .then(|| {
                value
                    .as_f64()
                    .and_then(|value| numeric_value(value / right))
            })
            .flatten(),
        MagicTransform::FloorDivide(right) => (*right != 0.0)
            .then(|| {
                value
                    .as_f64()
                    .and_then(|value| numeric_value((value / right).floor()))
            })
            .flatten(),
        MagicTransform::Modulo(right) => (*right != 0.0)
            .then(|| {
                value
                    .as_f64()
                    .and_then(|value| numeric_value(value % right))
            })
            .flatten(),
        MagicTransform::Power(right) => numeric_value(value.as_f64()?.powf(*right)),
        MagicTransform::BitAnd(right) => Some(serde_json::Value::from(value.as_i64()? & right)),
        MagicTransform::BitOr(right) => Some(serde_json::Value::from(value.as_i64()? | right)),
        MagicTransform::BitXor(right) => Some(serde_json::Value::from(value.as_i64()? ^ right)),
        MagicTransform::ShiftLeft(right) => value
            .as_i64()?
            .checked_shl(*right)
            .map(serde_json::Value::from),
        MagicTransform::ShiftRight(right) => value
            .as_i64()?
            .checked_shr(*right)
            .map(serde_json::Value::from),
        MagicTransform::Negate => numeric_value(-value.as_f64()?),
        MagicTransform::Custom(transform) => transform(value),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemberStatus {
    Creator,
    Administrator,
    Member,
    Restricted,
    Left,
    Kicked,
}

impl MemberStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creator => "creator",
            Self::Administrator => "administrator",
            Self::Member => "member",
            Self::Restricted => "restricted",
            Self::Left => "left",
            Self::Kicked => "kicked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemberStatusMarker {
    pub status: MemberStatus,
    pub is_member: Option<bool>,
}

impl MemberStatusMarker {
    pub const fn new(status: MemberStatus) -> Self {
        Self {
            status,
            is_member: None,
        }
    }

    /// Restricts the marker by current membership. This is meaningful for the
    /// Telegram `restricted` status, which may represent either state.
    pub const fn membership(mut self, value: bool) -> Self {
        self.is_member = Some(value);
        self
    }

    fn matches(self, member: &ChatMemberUnion) -> bool {
        self.status.as_str() == chat_member_status(member)
            && self
                .is_member
                .is_none_or(|expected| chat_member_is_member(member) == Some(expected))
    }
}

pub const CREATOR: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Creator);
pub const ADMINISTRATOR: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Administrator);
pub const MEMBER: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Member);
pub const RESTRICTED: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Restricted);
pub const LEFT: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Left);
pub const KICKED: MemberStatusMarker = MemberStatusMarker::new(MemberStatus::Kicked);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatusGroup(Vec<MemberStatusMarker>);

impl MemberStatusGroup {
    pub fn new(marker: MemberStatusMarker) -> Self {
        Self(vec![marker])
    }

    pub fn matches(&self, member: &ChatMemberUnion) -> bool {
        self.0.iter().any(|marker| marker.matches(member))
    }

    fn push_unique(mut self, marker: MemberStatusMarker) -> Self {
        if !self.0.contains(&marker) {
            self.0.push(marker);
        }
        self
    }
}

impl From<MemberStatusMarker> for MemberStatusGroup {
    fn from(value: MemberStatusMarker) -> Self {
        Self::new(value)
    }
}

impl BitOr<MemberStatusMarker> for MemberStatusMarker {
    type Output = MemberStatusGroup;

    fn bitor(self, rhs: MemberStatusMarker) -> Self::Output {
        MemberStatusGroup::new(self).push_unique(rhs)
    }
}

impl BitOr<MemberStatusMarker> for MemberStatusGroup {
    type Output = Self;

    fn bitor(self, rhs: MemberStatusMarker) -> Self::Output {
        self.push_unique(rhs)
    }
}

impl BitOr<MemberStatusGroup> for MemberStatusGroup {
    type Output = Self;

    fn bitor(mut self, rhs: MemberStatusGroup) -> Self::Output {
        for marker in rhs.0 {
            self = self.push_unique(marker);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatusTransition {
    pub old: MemberStatusGroup,
    pub new: MemberStatusGroup,
}

impl MemberStatusTransition {
    pub fn matches(&self, update: &ChatMemberUpdated) -> bool {
        self.old.matches(&update.old_chat_member) && self.new.matches(&update.new_chat_member)
    }
}

impl<Rhs: Into<MemberStatusGroup>> Shr<Rhs> for MemberStatusMarker {
    type Output = MemberStatusTransition;

    fn shr(self, rhs: Rhs) -> Self::Output {
        MemberStatusTransition {
            old: self.into(),
            new: rhs.into(),
        }
    }
}

impl<Rhs: Into<MemberStatusGroup>> Shr<Rhs> for MemberStatusGroup {
    type Output = MemberStatusTransition;

    fn shr(self, rhs: Rhs) -> Self::Output {
        MemberStatusTransition {
            old: self,
            new: rhs.into(),
        }
    }
}

impl StdNot for MemberStatusTransition {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self {
            old: self.new,
            new: self.old,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemberStatusRule {
    Group(MemberStatusGroup),
    Transition(MemberStatusTransition),
}

impl From<MemberStatusMarker> for MemberStatusRule {
    fn from(value: MemberStatusMarker) -> Self {
        Self::Group(value.into())
    }
}

impl From<MemberStatusGroup> for MemberStatusRule {
    fn from(value: MemberStatusGroup) -> Self {
        Self::Group(value)
    }
}

impl From<MemberStatusTransition> for MemberStatusRule {
    fn from(value: MemberStatusTransition) -> Self {
        Self::Transition(value)
    }
}

pub fn is_member() -> MemberStatusGroup {
    CREATOR | ADMINISTRATOR | MEMBER | RESTRICTED.membership(true)
}

pub fn is_admin() -> MemberStatusGroup {
    CREATOR | ADMINISTRATOR
}

pub fn is_not_member() -> MemberStatusGroup {
    LEFT | KICKED | RESTRICTED.membership(false)
}

pub fn join_transition() -> MemberStatusTransition {
    is_not_member() >> is_member()
}

pub fn leave_transition() -> MemberStatusTransition {
    !join_transition()
}

pub fn promoted_transition() -> MemberStatusTransition {
    (MEMBER | RESTRICTED | LEFT | KICKED) >> ADMINISTRATOR
}

/// Matches `chat_member` and `my_chat_member` status changes.
#[derive(Debug, Clone)]
pub struct ChatMemberUpdatedFilter(MemberStatusRule);

impl ChatMemberUpdatedFilter {
    pub fn new(rule: impl Into<MemberStatusRule>) -> Self {
        Self(rule.into())
    }
}

impl Filter for ChatMemberUpdatedFilter {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            let Some(update) = context
                .update
                .my_chat_member
                .as_ref()
                .or(context.update.chat_member.as_ref())
            else {
                return false;
            };
            match &self.0 {
                MemberStatusRule::Group(group) => group.matches(&update.new_chat_member),
                MemberStatusRule::Transition(transition) => transition.matches(update),
            }
        })
    }
}

fn chat_member_status(member: &ChatMemberUnion) -> &str {
    match member {
        ChatMemberUnion::ChatMemberAdministrator(value) => &value.status,
        ChatMemberUnion::ChatMemberBanned(value) => &value.status,
        ChatMemberUnion::ChatMemberLeft(value) => &value.status,
        ChatMemberUnion::ChatMemberMember(value) => &value.status,
        ChatMemberUnion::ChatMemberOwner(value) => &value.status,
        ChatMemberUnion::ChatMemberRestricted(value) => &value.status,
    }
}

fn chat_member_is_member(member: &ChatMemberUnion) -> Option<bool> {
    match member {
        ChatMemberUnion::ChatMemberRestricted(value) => Some(value.is_member),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(status: &str, is_member: Option<bool>) -> ChatMemberUnion {
        let mut value = serde_json::json!({
            "status": status,
            "user": {"id": 1, "is_bot": false, "first_name": "Ada"}
        });
        if let Some(is_member) = is_member {
            value["is_member"] = serde_json::json!(is_member);
            for permission in [
                "can_send_messages",
                "can_send_audios",
                "can_send_documents",
                "can_send_photos",
                "can_send_videos",
                "can_send_video_notes",
                "can_send_voice_notes",
                "can_send_polls",
                "can_send_other_messages",
                "can_add_web_page_previews",
                "can_react_to_messages",
                "can_edit_tag",
                "can_change_info",
                "can_invite_users",
                "can_pin_messages",
                "can_manage_topics",
            ] {
                value[permission] = serde_json::json!(false);
            }
            value["until_date"] = serde_json::json!(0);
        }
        if status == "creator" {
            value["is_anonymous"] = serde_json::json!(false);
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn member_groups_preserve_restricted_membership_semantics() {
        assert!(is_member().matches(&member("member", None)));
        assert!(is_member().matches(&member("restricted", Some(true))));
        assert!(!is_member().matches(&member("restricted", Some(false))));
        assert!(is_not_member().matches(&member("restricted", Some(false))));
        assert!(is_admin().matches(&member("creator", None)));
    }

    #[test]
    fn member_transition_operators_match_aiogram_rules() {
        let update: ChatMemberUpdated = serde_json::from_value(serde_json::json!({
            "chat": {"id": -1, "type": "group"},
            "from": {"id": 2, "is_bot": false, "first_name": "Grace"},
            "date": 1,
            "old_chat_member": {
                "status": "left",
                "user": {"id": 1, "is_bot": false, "first_name": "Ada"}
            },
            "new_chat_member": {
                "status": "member",
                "user": {"id": 1, "is_bot": false, "first_name": "Ada"}
            }
        }))
        .unwrap();
        assert!(join_transition().matches(&update));
        assert!(!leave_transition().matches(&update));
        assert!((LEFT >> MEMBER).matches(&update));
    }

    #[tokio::test]
    async fn command_ignore_case_uses_unicode_casefold_and_regex_matches_from_start() {
        fn update(id: i64, text: &str) -> crate::types::Update {
            serde_json::from_value(serde_json::json!({
                "update_id": id,
                "message": {
                    "message_id": id,
                    "date": 1,
                    "chat": {"id": 1, "type": "private"},
                    "text": text
                }
            }))
            .unwrap()
        }

        let unicode_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let regex_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut router = crate::Router::new();
        let calls = unicode_calls.clone();
        router.message(Command::new("STRASSE").ignore_case(true), move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        });
        let calls = regex_calls.clone();
        router.message(Command::regex("foo").unwrap(), move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        });
        let mut dispatcher = crate::Dispatcher::new();
        dispatcher.include_router(router);
        let bot = crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();

        assert!(
            dispatcher
                .feed_update(bot.clone(), update(1, "/straße"))
                .await
                .unwrap()
        );
        assert!(
            !dispatcher
                .feed_update(bot.clone(), update(2, "/xfoo"))
                .await
                .unwrap()
        );
        assert!(
            dispatcher
                .feed_update(bot, update(3, "/foobar"))
                .await
                .unwrap()
        );
        assert_eq!(unicode_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(regex_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn magic_field_applies_transforms_truthiness_and_regex_modes() {
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let transformed_calls = Arc::new(AtomicUsize::new(0));
        let transformed_capture = transformed_calls.clone();
        let selector_calls = Arc::new(AtomicUsize::new(0));
        let selector_capture = selector_calls.clone();
        let regex_matches = Arc::new(Mutex::new(Vec::<MagicRegexMatch>::new()));
        let regex_capture = regex_matches.clone();

        let mut router = crate::Router::new();
        router.message(
            field("text")
                .casefold()
                .equals("hello")
                .and(field("text").len().greater_or_equal(5.0)),
            move |_| {
                let transformed_capture = transformed_capture.clone();
                async move {
                    transformed_capture.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        router.message(
            field("text").regex(r"(?P<digits>\d+)").unwrap(),
            move |context| {
                let regex_capture = regex_capture.clone();
                async move {
                    regex_capture.lock().unwrap().push(
                        context
                            .dependency::<MagicRegexMatch>()
                            .unwrap()
                            .as_ref()
                            .clone(),
                    );
                    Ok(())
                }
            },
        );
        router.message(
            field("items")
                .index(1)
                .equals(2.0)
                .and(field("items").any(|value| value.as_i64() == Some(3)))
                .and((field("mask") & 4).equals(4)),
            move |_| {
                let selector_capture = selector_capture.clone();
                async move {
                    selector_capture.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );

        let mut dispatcher = crate::Dispatcher::new();
        dispatcher.include_router(router);
        let bot = crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
        for (update_id, text) in [
            (1, "HeLLo"),
            (2, "123"),
            (3, "x123"),
            (4, ""),
            (5, "Straße"),
        ] {
            let update = serde_json::from_value(serde_json::json!({
                "update_id": update_id,
                "message": {
                    "message_id": update_id,
                    "date": 1,
                    "chat": {"id": 1, "type": "private"},
                    "text": text
                }
            }))
            .unwrap();
            dispatcher.feed_update(bot.clone(), update).await.unwrap();
        }
        let collection_update = serde_json::from_value(serde_json::json!({
            "update_id": 6,
            "message": {
                "message_id": 6,
                "date": 1,
                "chat": {"id": 1, "type": "private"},
                "text": "collection",
                "items": [1, 2, 3],
                "mask": 5
            }
        }))
        .unwrap();
        dispatcher
            .feed_update(bot.clone(), collection_update)
            .await
            .unwrap();

        assert_eq!(transformed_calls.load(Ordering::SeqCst), 1);
        assert_eq!(selector_calls.load(Ordering::SeqCst), 1);
        {
            let matches = regex_matches.lock().unwrap();
            assert_eq!(
                matches.len(),
                1,
                "default regexp mode must match from start"
            );
            assert_eq!(matches[0].matched, "123");
            assert_eq!(matches[0].named["digits"], "123");
        }

        assert_eq!(
            apply_magic_transform(
                &MagicTransform::Casefold,
                serde_json::Value::String("Straße".to_owned()),
            ),
            Some(serde_json::Value::String("strasse".to_owned()))
        );
        assert_eq!(
            apply_magic_transform(
                &MagicTransform::FloorDivide(2.0),
                serde_json::Value::from(-5),
            ),
            Some(serde_json::json!(-3.0))
        );

        let empty_update = serde_json::from_value(serde_json::json!({
            "update_id": 10,
            "message": {
                "message_id": 10,
                "date": 1,
                "chat": {"id": 1, "type": "private"},
                "text": ""
            }
        }))
        .unwrap();
        let context_checked = Arc::new(AtomicUsize::new(0));
        let checked = context_checked.clone();
        let mut truthy_router = crate::Router::new();
        truthy_router.message(field("text").exists(), move |_| {
            let checked = checked.clone();
            async move {
                checked.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let mut truthy_dispatcher = crate::Dispatcher::new();
        truthy_dispatcher.include_router(truthy_router);
        truthy_dispatcher
            .feed_update(bot, empty_update)
            .await
            .unwrap();
        assert_eq!(context_checked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn magic_regex_find_modes_inject_every_match() {
        use std::sync::Mutex;

        let captured = Arc::new(Mutex::new(None::<MagicRegexMatches>));
        let captured_handler = captured.clone();
        let mut router = crate::Router::new();
        router.message(
            field("text").regex_find_all(r"(?P<number>\d+)").unwrap(),
            move |context| {
                let captured_handler = captured_handler.clone();
                async move {
                    *captured_handler.lock().unwrap() = Some(
                        context
                            .dependency::<MagicRegexMatches>()
                            .unwrap()
                            .as_ref()
                            .clone(),
                    );
                    Ok(())
                }
            },
        );
        let mut dispatcher = crate::Dispatcher::new();
        dispatcher.include_router(router);
        let update = serde_json::from_value(serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "date": 1,
                "chat": {"id": 1, "type": "private"},
                "text": "items 12 and 345"
            }
        }))
        .unwrap();
        dispatcher
            .feed_update(
                crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
                update,
            )
            .await
            .unwrap();

        let captured = captured.lock().unwrap();
        let matches = captured.as_ref().unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches.matches[0].named["number"], "12");
        assert_eq!(matches.matches[1].matched, "345");
    }
}
