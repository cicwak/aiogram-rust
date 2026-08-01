use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Notify, Semaphore, watch};
use tokio::task::JoinSet;

use crate::bot::Bot;
use crate::client::BotRequest;
use crate::error::{Error, Result};
use crate::filters::Filter;
use crate::fsm::FsmMiddleware;
use crate::methods::{
    AnswerCallbackQuery, EditMessageText, GetUpdates, MessageOrBool, SendMessage, TelegramMethod,
};
use crate::types::{
    CallbackQuery, Chat, ChatBoostSourceUnion, MaybeInaccessibleMessageUnion, Message,
    ReplyParameters, Update, User,
};
use crate::utils::backoff::{Backoff, BackoffConfig};

type HandlerFuture = Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;
type Handler = Arc<dyn Fn(UpdateContext) -> HandlerFuture + Send + Sync>;
type LifecycleHandler = Arc<dyn Fn(Bot) -> HandlerFuture + Send + Sync>;
type DispatchFuture<'a> = Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>>;

#[derive(Default)]
struct PollingControl {
    shutdown: Mutex<Option<watch::Sender<bool>>>,
    stopped: Notify,
}

impl PollingControl {
    fn start(&self) -> Result<(watch::Sender<bool>, watch::Receiver<bool>)> {
        let mut shutdown = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if shutdown.is_some() {
            return Err(Error::PollingAlreadyStarted);
        }
        let (sender, receiver) = watch::channel(false);
        *shutdown = Some(sender.clone());
        Ok((sender, receiver))
    }

    fn finish(&self) {
        self.shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.stopped.notify_waiters();
    }

    fn is_running(&self) -> bool {
        self.shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    async fn stop(&self) -> Result<()> {
        let sender = self
            .shutdown
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .ok_or(Error::PollingNotStarted)?;
        let _ = sender.send(true);
        self.wait_stopped().await;
        Ok(())
    }

    async fn wait_stopped(&self) {
        loop {
            let notified = self.stopped.notified();
            if !self.is_running() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(unix)]
async fn polling_shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate()).ok();
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = async {
            if let Some(signal) = &mut terminate {
                signal.recv().await;
            } else {
                std::future::pending::<()>().await;
            }
        } => {}
    }
}

#[cfg(not(unix))]
async fn polling_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[derive(Clone, Default)]
struct Dependencies(Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>);

impl Dependencies {
    fn new(values: HashMap<TypeId, Arc<dyn Any + Send + Sync>>) -> Self {
        Self(Arc::new(RwLock::new(values)))
    }

    fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&TypeId::of::<T>())
            .cloned()?
            .downcast()
            .ok()
    }

    fn with<T: Any + Send + Sync>(&self, value: T) -> Self {
        let mut dependencies = self.snapshot();
        dependencies.insert(TypeId::of::<T>(), Arc::new(value));
        Self::new(dependencies)
    }

    fn insert<T: Any + Send + Sync>(&self, value: T) {
        self.0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(TypeId::of::<T>(), Arc::new(value));
    }

    fn fork(&self) -> Self {
        Self::new(self.snapshot())
    }

    fn snapshot(&self) -> HashMap<TypeId, Arc<dyn Any + Send + Sync>> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Typed, named metadata attached to a handler registration.
///
/// Flags are visible to filters, middleware, and the handler through
/// [`UpdateContext::handler_flags`]. They are the Rust equivalent of aiogram's
/// handler flag dictionary without sacrificing static typing.
#[derive(Clone, Default)]
pub struct HandlerFlags(Arc<HashMap<String, Arc<dyn Any + Send + Sync>>>);

impl HandlerFlags {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with<T: Any + Send + Sync>(mut self, name: impl Into<String>, value: T) -> Self {
        self.insert(name, value);
        self
    }

    pub fn insert<T: Any + Send + Sync>(&mut self, name: impl Into<String>, value: T) {
        Arc::make_mut(&mut self.0).insert(name.into(), Arc::new(value));
    }

    pub fn get<T: Any + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        self.0.get(name).cloned()?.downcast().ok()
    }

    pub fn get_cloned<T: Any + Clone + Send + Sync>(&self, name: &str) -> Option<T> {
        self.get::<T>(name).map(|value| value.as_ref().clone())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.0.contains_key(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Debug for HandlerFlags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = self.names().collect::<Vec<_>>();
        names.sort_unstable();
        formatter
            .debug_struct("HandlerFlags")
            .field("names", &names)
            .finish()
    }
}

/// Chat/user coordinates resolved from every Telegram update variant.
#[derive(Debug, Clone, Default)]
pub struct EventContext {
    pub chat: Option<Chat>,
    pub user: Option<User>,
    pub thread_id: Option<i64>,
    pub business_connection_id: Option<String>,
}

impl EventContext {
    pub fn chat_id(&self) -> Option<i64> {
        self.chat.as_ref().map(|chat| chat.id)
    }

    pub fn user_id(&self) -> Option<i64> {
        self.user.as_ref().map(|user| user.id)
    }

    pub fn resolve(update: &Update) -> Self {
        let message_context = |message: &Message, user: bool| Self {
            chat: Some(message.chat.as_ref().clone()),
            user: user.then(|| message.from_user.as_ref().cloned()).flatten(),
            thread_id: message.message_thread_id,
            business_connection_id: message.business_connection_id.clone(),
        };
        if let Some(message) = update.message.as_deref() {
            return message_context(message, true);
        }
        if let Some(message) = update.edited_message.as_deref() {
            return message_context(message, true);
        }
        if let Some(message) = update.channel_post.as_deref() {
            return message_context(message, false);
        }
        if let Some(message) = update.edited_channel_post.as_deref() {
            return message_context(message, false);
        }
        if let Some(message) = update.business_message.as_deref() {
            return message_context(message, true);
        }
        if let Some(message) = update.edited_business_message.as_deref() {
            return message_context(message, true);
        }
        if let Some(message) = update.guest_message.as_deref() {
            return message_context(message, true);
        }
        if let Some(query) = &update.callback_query {
            let (chat, thread_id, business_connection_id) = match query.message.as_ref() {
                Some(MaybeInaccessibleMessageUnion::Message(message)) => (
                    Some(message.chat.as_ref().clone()),
                    message.message_thread_id,
                    message.business_connection_id.clone(),
                ),
                Some(MaybeInaccessibleMessageUnion::InaccessibleMessage(message)) => {
                    (Some(message.chat.as_ref().clone()), None, None)
                }
                None => (None, None, None),
            };
            return Self {
                chat,
                user: Some(query.from_user.clone()),
                thread_id,
                business_connection_id,
            };
        }
        if let Some(query) = &update.inline_query {
            return Self::user(query.from_user.clone());
        }
        if let Some(result) = &update.chosen_inline_result {
            return Self::user(result.from_user.clone());
        }
        if let Some(query) = &update.shipping_query {
            return Self::user(query.from_user.clone());
        }
        if let Some(query) = &update.pre_checkout_query {
            return Self::user(query.from_user.clone());
        }
        if let Some(purchase) = &update.purchased_paid_media {
            return Self::user(purchase.from_user.clone());
        }
        if let Some(answer) = &update.poll_answer {
            return Self {
                chat: answer.voter_chat.as_deref().cloned(),
                user: answer.user.clone(),
                ..Self::default()
            };
        }
        if let Some(member) = update
            .my_chat_member
            .as_ref()
            .or(update.chat_member.as_ref())
        {
            return Self {
                chat: Some(member.chat.as_ref().clone()),
                user: Some(member.from_user.clone()),
                ..Self::default()
            };
        }
        if let Some(request) = &update.chat_join_request {
            return Self {
                chat: Some(request.chat.as_ref().clone()),
                user: Some(request.from_user.clone()),
                ..Self::default()
            };
        }
        if let Some(reaction) = &update.message_reaction {
            return Self {
                chat: Some(reaction.chat.as_ref().clone()),
                user: reaction.user.clone(),
                ..Self::default()
            };
        }
        if let Some(reaction) = &update.message_reaction_count {
            return Self::chat(reaction.chat.as_ref().clone());
        }
        if let Some(boost) = &update.chat_boost {
            let user = match &boost.boost.source {
                ChatBoostSourceUnion::ChatBoostSourcePremium(source) => Some(source.user.clone()),
                _ => None,
            };
            return Self {
                chat: Some(boost.chat.as_ref().clone()),
                user,
                ..Self::default()
            };
        }
        if let Some(boost) = &update.removed_chat_boost {
            return Self::chat(boost.chat.as_ref().clone());
        }
        if let Some(deleted) = &update.deleted_business_messages {
            return Self {
                chat: Some(deleted.chat.as_ref().clone()),
                business_connection_id: Some(deleted.business_connection_id.clone()),
                ..Self::default()
            };
        }
        if let Some(connection) = &update.business_connection {
            return Self {
                user: Some(connection.user.clone()),
                business_connection_id: Some(connection.id.clone()),
                ..Self::default()
            };
        }
        if let Some(managed) = &update.managed_bot {
            return Self::user(managed.user.clone());
        }
        if let Some(subscription) = &update.subscription {
            return Self::user(subscription.user.clone());
        }
        Self::default()
    }

    fn user(user: User) -> Self {
        Self {
            user: Some(user),
            ..Self::default()
        }
    }

    fn chat(chat: Chat) -> Self {
        Self {
            chat: Some(chat),
            ..Self::default()
        }
    }
}

/// Data passed to filters, middleware, and handlers for one update.
#[derive(Clone)]
pub struct UpdateContext {
    pub bot: Bot,
    pub update: Arc<Update>,
    dependencies: Dependencies,
    webhook_response: Option<Arc<Mutex<Option<BotRequest>>>>,
    handler_error: Option<Arc<Error>>,
}

impl UpdateContext {
    pub fn event_type(&self) -> Option<&'static str> {
        if self.handler_error.is_some() {
            Some("error")
        } else {
            self.update.event_type()
        }
    }

    pub fn message(&self) -> Option<&Message> {
        self.update.message_event()
    }

    pub fn callback_query(&self) -> Option<&CallbackQuery> {
        self.update.callback_query.as_ref()
    }

    pub fn event_context(&self) -> Option<Arc<EventContext>> {
        self.dependency::<EventContext>()
    }

    pub fn dependency<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.dependencies.get::<T>()
    }

    pub fn handler_flags(&self) -> Option<Arc<HandlerFlags>> {
        self.dependency::<HandlerFlags>()
    }

    pub fn handler_flag<T: Any + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        self.handler_flags()?.get(name)
    }

    /// Error raised by a handler or middleware while dispatching this update.
    /// It is available inside routes registered with [`Router::error`].
    pub fn error(&self) -> Option<&Error> {
        self.handler_error.as_deref()
    }

    pub fn with_dependency<T: Any + Send + Sync>(&self, value: T) -> Self {
        let mut context = self.clone();
        context.dependencies = self.dependencies.with(value);
        context
    }

    /// Injects data produced by a filter for the current route attempt.
    pub fn inject_dependency<T: Any + Send + Sync>(&self, value: T) {
        self.dependencies.insert(value);
    }

    fn fork_dependencies(&self) -> Self {
        let mut context = self.clone();
        context.dependencies = self.dependencies.fork();
        context
    }

    fn with_error(&self, error: Arc<Error>) -> Self {
        let mut context = self.clone();
        context.handler_error = Some(error);
        context
    }

    /// Sends a text reply to the chat associated with the current message.
    pub async fn answer(&self, text: impl Into<String>) -> Result<Message> {
        let message = self
            .message()
            .ok_or_else(|| Error::Handler("answer() requires a message update".to_owned()))?;
        self.bot.send_message(message.chat.id, text).await
    }

    /// Replies to the current message and preserves Telegram's reply context.
    pub async fn reply(&self, text: impl Into<String>) -> Result<Message> {
        let message = self
            .message()
            .ok_or_else(|| Error::Handler("reply() requires a message update".to_owned()))?;
        self.bot
            .execute(
                &SendMessage::new(message.chat.id, text)
                    .reply_parameters(ReplyParameters::new().message_id(message.message_id)),
            )
            .await
    }

    /// Edits the text of the current message.
    pub async fn edit_message_text(&self, text: impl Into<String>) -> Result<MessageOrBool> {
        let message = self.message().ok_or_else(|| {
            Error::Handler("edit_message_text() requires a message update".to_owned())
        })?;
        self.bot
            .edit_message_text(
                EditMessageText::new()
                    .chat_id(message.chat.id)
                    .message_id(message.message_id)
                    .text(text),
            )
            .await
    }

    pub async fn delete_message(&self) -> Result<bool> {
        let message = self.message().ok_or_else(|| {
            Error::Handler("delete_message() requires a message update".to_owned())
        })?;
        self.bot
            .delete_message(message.chat.id, message.message_id)
            .await
    }

    /// Answers the current callback query with optional notification text.
    pub async fn answer_callback(&self, text: Option<impl Into<String>>) -> Result<bool> {
        let query = self.callback_query().ok_or_else(|| {
            Error::Handler("answer_callback() requires a callback query update".to_owned())
        })?;
        let mut method = AnswerCallbackQuery::new(query.id.clone());
        if let Some(text) = text {
            method = method.text(text);
        }
        self.bot.execute(&method).await
    }

    /// Stores a Bot API method as the direct JSON or multipart response to a webhook.
    pub fn answer_webhook<M: TelegramMethod>(&self, method: &M) -> Result<()> {
        let response = self.webhook_response.as_ref().ok_or_else(|| {
            Error::Handler("answer_webhook() is only available for webhook updates".to_owned())
        })?;
        let request = self.bot.prepare_request(method)?;
        *response
            .lock()
            .map_err(|_| Error::Handler("webhook response lock poisoned".to_owned()))? =
            Some(request);
        Ok(())
    }
}

/// The remaining middleware/handler chain.
#[derive(Clone)]
pub struct Next {
    middlewares: Arc<Vec<Arc<dyn Middleware>>>,
    handler: Handler,
    index: usize,
}

impl Next {
    fn new(middlewares: Vec<Arc<dyn Middleware>>, handler: Handler) -> Self {
        Self {
            middlewares: Arc::new(middlewares),
            handler,
            index: 0,
        }
    }

    pub async fn run(self, context: UpdateContext) -> Result<()> {
        if let Some(middleware) = self.middlewares.get(self.index).cloned() {
            let next = Self {
                index: self.index + 1,
                ..self
            };
            middleware.handle(context, next).await
        } else {
            (self.handler)(context).await
        }
    }
}

/// Incoming update middleware. It can modify dependencies indirectly, perform
/// checks, skip `next`, or wrap the downstream handler with before/after work.
#[async_trait]
pub trait Middleware: Send + Sync {
    async fn handle(&self, context: UpdateContext, next: Next) -> Result<()>;
}

/// Stateful or class-style update handler. This is the Rust counterpart of
/// aiogram's `BaseHandler` subclasses; closure handlers remain available for
/// concise routes.
#[async_trait]
pub trait ClassHandler: Send + Sync + 'static {
    async fn handle(&self, context: UpdateContext) -> Result<()>;
}

/// The remaining router-level outer-middleware chain.
pub struct OuterNext<'a> {
    router: &'a Router,
    middlewares: Arc<Vec<Arc<dyn OuterMiddleware>>>,
    inherited_middlewares: Vec<Arc<dyn Middleware>>,
    index: usize,
}

impl<'a> OuterNext<'a> {
    fn new(
        router: &'a Router,
        inherited_middlewares: Vec<Arc<dyn Middleware>>,
        middlewares: Vec<Arc<dyn OuterMiddleware>>,
    ) -> Self {
        Self {
            router,
            middlewares: Arc::new(middlewares),
            inherited_middlewares,
            index: 0,
        }
    }

    pub async fn run(self, context: UpdateContext) -> Result<bool> {
        if let Some(middleware) = self.middlewares.get(self.index).cloned() {
            let next = Self {
                index: self.index + 1,
                ..self
            };
            middleware.handle(context, next).await
        } else {
            self.router
                .dispatch_inner(context, self.inherited_middlewares)
                .await
        }
    }
}

/// Router middleware that wraps filter evaluation, handlers and child routers.
#[async_trait]
pub trait OuterMiddleware: Send + Sync {
    async fn handle(&self, context: UpdateContext, next: OuterNext<'_>) -> Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EventKind {
    Any,
    Named(String),
}

#[derive(Clone)]
struct Route {
    event: EventKind,
    filters: Vec<Arc<dyn Filter>>,
    flags: HandlerFlags,
    handler: Handler,
}

/// A composable collection of update handlers, similar to aiogram's Router.
#[derive(Default, Clone)]
pub struct Router {
    name: Option<String>,
    routes: Vec<Route>,
    root_filters: HashMap<String, Vec<Arc<dyn Filter>>>,
    middlewares: Vec<Arc<dyn Middleware>>,
    event_middlewares: HashMap<String, Vec<Arc<dyn Middleware>>>,
    outer_middlewares: Vec<Arc<dyn OuterMiddleware>>,
    event_outer_middlewares: HashMap<String, Vec<Arc<dyn OuterMiddleware>>>,
    startup_handlers: Vec<LifecycleHandler>,
    shutdown_handlers: Vec<LifecycleHandler>,
    sub_routers: Vec<Router>,
}

macro_rules! event_shortcuts {
    ($($name:ident => $event:literal),+ $(,)?) => {
        $(
            pub fn $name<F, Fut>(
                &mut self,
                filter: impl Filter + 'static,
                handler: F,
            ) -> &mut Self
            where
                F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
                Fut: Future<Output = Result<()>> + Send + 'static,
            {
                self.event($event, filter, handler)
            }
        )+
    };
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn include_router(&mut self, router: Router) -> &mut Self {
        self.sub_routers.push(router);
        self
    }

    pub fn include_routers(&mut self, routers: impl IntoIterator<Item = Router>) -> &mut Self {
        self.sub_routers.extend(routers);
        self
    }

    /// Adds a filter that gates every handler and child router for this event,
    /// corresponding to `router.<event>.filter(...)` in aiogram.
    pub fn event_filter(
        &mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
    ) -> &mut Self {
        self.root_filters
            .entry(event_type.into())
            .or_default()
            .push(Arc::new(filter));
        self
    }

    pub fn message_filter(&mut self, filter: impl Filter + 'static) -> &mut Self {
        self.event_filter("message", filter)
    }

    pub fn middleware(&mut self, middleware: impl Middleware + 'static) -> &mut Self {
        self.middlewares.push(Arc::new(middleware));
        self
    }

    pub fn event_middleware(
        &mut self,
        event_type: impl Into<String>,
        middleware: impl Middleware + 'static,
    ) -> &mut Self {
        self.event_middlewares
            .entry(event_type.into())
            .or_default()
            .push(Arc::new(middleware));
        self
    }

    pub fn message_middleware(&mut self, middleware: impl Middleware + 'static) -> &mut Self {
        self.event_middleware("message", middleware)
    }

    pub fn outer_middleware(&mut self, middleware: impl OuterMiddleware + 'static) -> &mut Self {
        self.outer_middlewares.push(Arc::new(middleware));
        self
    }

    pub fn event_outer_middleware(
        &mut self,
        event_type: impl Into<String>,
        middleware: impl OuterMiddleware + 'static,
    ) -> &mut Self {
        self.event_outer_middlewares
            .entry(event_type.into())
            .or_default()
            .push(Arc::new(middleware));
        self
    }

    pub fn message_outer_middleware(
        &mut self,
        middleware: impl OuterMiddleware + 'static,
    ) -> &mut Self {
        self.event_outer_middleware("message", middleware)
    }

    pub fn startup<F, Fut>(&mut self, handler: F) -> &mut Self
    where
        F: Fn(Bot) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.startup_handlers
            .push(Arc::new(move |bot| Box::pin(handler(bot))));
        self
    }

    pub fn shutdown<F, Fut>(&mut self, handler: F) -> &mut Self
    where
        F: Fn(Bot) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.shutdown_handlers
            .push(Arc::new(move |bot| Box::pin(handler(bot))));
        self
    }

    pub fn message<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event("message", filter, handler)
    }

    pub fn message_handler(
        &mut self,
        filter: impl Filter + 'static,
        handler: impl ClassHandler,
    ) -> &mut Self {
        self.event_handler("message", filter, handler)
    }

    pub fn message_with_flags<F, Fut>(
        &mut self,
        filter: impl Filter + 'static,
        flags: HandlerFlags,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event_with_flags("message", filter, flags, handler)
    }

    pub fn message_filters<F, Fut>(
        &mut self,
        filters: Vec<Arc<dyn Filter>>,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route(EventKind::Named("message".to_owned()), filters, handler)
    }

    pub fn message_filters_with_flags<F, Fut>(
        &mut self,
        filters: Vec<Arc<dyn Filter>>,
        flags: HandlerFlags,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route_with_flags(
            EventKind::Named("message".to_owned()),
            filters,
            flags,
            handler,
        )
    }

    pub fn edited_message<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event("edited_message", filter, handler)
    }

    pub fn channel_post<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event("channel_post", filter, handler)
    }

    pub fn callback_query<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event("callback_query", filter, handler)
    }

    pub fn callback_query_handler(
        &mut self,
        filter: impl Filter + 'static,
        handler: impl ClassHandler,
    ) -> &mut Self {
        self.event_handler("callback_query", filter, handler)
    }

    event_shortcuts! {
        edited_channel_post => "edited_channel_post",
        business_connection => "business_connection",
        business_message => "business_message",
        edited_business_message => "edited_business_message",
        deleted_business_messages => "deleted_business_messages",
        guest_message => "guest_message",
        message_reaction => "message_reaction",
        message_reaction_count => "message_reaction_count",
        inline_query => "inline_query",
        chosen_inline_result => "chosen_inline_result",
        shipping_query => "shipping_query",
        pre_checkout_query => "pre_checkout_query",
        purchased_paid_media => "purchased_paid_media",
        poll => "poll",
        poll_answer => "poll_answer",
        my_chat_member => "my_chat_member",
        chat_member => "chat_member",
        chat_join_request => "chat_join_request",
        chat_boost => "chat_boost",
        removed_chat_boost => "removed_chat_boost",
        managed_bot => "managed_bot",
        subscription => "subscription",
    }

    pub fn error<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event("error", filter, handler)
    }

    /// Registers any Telegram update type supported by the pinned schema.
    pub fn event<F, Fut>(
        &mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.event_with_flags(event_type, filter, HandlerFlags::default(), handler)
    }

    pub fn event_handler(
        &mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
        handler: impl ClassHandler,
    ) -> &mut Self {
        let handler = Arc::new(handler);
        self.event(event_type, filter, move |context| {
            let handler = handler.clone();
            async move { handler.handle(context).await }
        })
    }

    pub fn event_with_flags<F, Fut>(
        &mut self,
        event_type: impl Into<String>,
        filter: impl Filter + 'static,
        flags: HandlerFlags,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route_with_flags(
            EventKind::Named(event_type.into()),
            vec![Arc::new(filter)],
            flags,
            handler,
        )
    }

    pub fn update<F, Fut>(&mut self, filter: impl Filter + 'static, handler: F) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route(EventKind::Any, vec![Arc::new(filter)], handler)
    }

    pub fn update_with_flags<F, Fut>(
        &mut self,
        filter: impl Filter + 'static,
        flags: HandlerFlags,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route_with_flags(EventKind::Any, vec![Arc::new(filter)], flags, handler)
    }

    fn route<F, Fut>(
        &mut self,
        event: EventKind,
        filters: Vec<Arc<dyn Filter>>,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.route_with_flags(event, filters, HandlerFlags::default(), handler)
    }

    fn route_with_flags<F, Fut>(
        &mut self,
        event: EventKind,
        filters: Vec<Arc<dyn Filter>>,
        mut flags: HandlerFlags,
        handler: F,
    ) -> &mut Self
    where
        F: Fn(UpdateContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        for filter in &filters {
            filter.update_handler_flags(&mut flags);
        }
        self.routes.push(Route {
            event,
            filters,
            flags,
            handler: Arc::new(move |context| Box::pin(handler(context))),
        });
        self
    }

    fn dispatch<'a>(
        &'a self,
        context: UpdateContext,
        inherited_middlewares: Vec<Arc<dyn Middleware>>,
    ) -> DispatchFuture<'a> {
        Box::pin(async move {
            let mut outer_middlewares = self.outer_middlewares.clone();
            if let Some(event_type) = context.event_type()
                && let Some(event_middlewares) = self.event_outer_middlewares.get(event_type)
            {
                outer_middlewares.extend(event_middlewares.iter().cloned());
            }
            OuterNext::new(self, inherited_middlewares, outer_middlewares)
                .run(context)
                .await
        })
    }

    fn dispatch_inner<'a>(
        &'a self,
        context: UpdateContext,
        mut inherited_middlewares: Vec<Arc<dyn Middleware>>,
    ) -> DispatchFuture<'a> {
        Box::pin(async move {
            inherited_middlewares.extend(self.middlewares.iter().cloned());
            if let Some(event_type) = context.event_type() {
                if let Some(filters) = self.root_filters.get(event_type) {
                    for filter in filters {
                        if !filter.check(&context).await {
                            return Ok(false);
                        }
                    }
                }
                if let Some(event_middlewares) = self.event_middlewares.get(event_type) {
                    inherited_middlewares.extend(event_middlewares.iter().cloned());
                }
            }
            for route in &self.routes {
                if !event_matches(&route.event, &context) {
                    continue;
                }
                let route_context = context.fork_dependencies();
                route_context.inject_dependency(route.flags.clone());
                let mut matched = true;
                for filter in &route.filters {
                    if !filter.check(&route_context).await {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    let next = Next::new(inherited_middlewares.clone(), route.handler.clone());
                    match next.run(route_context).await {
                        Ok(()) | Err(Error::CancelHandler) => return Ok(true),
                        Err(Error::SkipHandler) => continue,
                        Err(error) => return Err(error),
                    }
                }
            }
            for router in &self.sub_routers {
                if router
                    .dispatch(context.clone(), inherited_middlewares.clone())
                    .await?
                {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    fn collect_used_update_types(&self, output: &mut BTreeSet<String>) -> bool {
        let mut has_catch_all = false;
        for route in &self.routes {
            match &route.event {
                EventKind::Named(event) => {
                    if event != "error" {
                        output.insert(event.clone());
                    }
                }
                EventKind::Any => has_catch_all = true,
            }
        }
        for router in &self.sub_routers {
            has_catch_all |= router.collect_used_update_types(output);
        }
        has_catch_all
    }

    fn emit_lifecycle<'a>(
        &'a self,
        bot: Bot,
        startup: bool,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let handlers = if startup {
                &self.startup_handlers
            } else {
                &self.shutdown_handlers
            };
            for handler in handlers {
                handler(bot.clone()).await?;
            }
            for router in &self.sub_routers {
                router.emit_lifecycle(bot.clone(), startup).await?;
            }
            Ok(())
        })
    }
}

fn event_matches(kind: &EventKind, context: &UpdateContext) -> bool {
    match kind {
        EventKind::Any => context.handler_error.is_none(),
        EventKind::Named(expected) => context.event_type() == Some(expected.as_str()),
    }
}

/// Receives updates and dispatches each one through included routers.
#[derive(Clone)]
pub struct Dispatcher {
    routers: Vec<Router>,
    dependencies: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    allowed_updates: Option<Vec<String>>,
    polling_timeout: Duration,
    backoff_config: BackoffConfig,
    handle_as_tasks: bool,
    tasks_concurrency_limit: Option<usize>,
    fsm_middleware: Option<FsmMiddleware>,
    startup_handlers: Vec<LifecycleHandler>,
    shutdown_handlers: Vec<LifecycleHandler>,
    polling_control: Arc<PollingControl>,
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self {
            routers: Vec::new(),
            dependencies: HashMap::new(),
            allowed_updates: None,
            polling_timeout: Duration::from_secs(30),
            backoff_config: BackoffConfig::default(),
            handle_as_tasks: true,
            tasks_concurrency_limit: None,
            fsm_middleware: None,
            startup_handlers: Vec::new(),
            shutdown_handlers: Vec::new(),
            polling_control: Arc::new(PollingControl::default()),
        }
    }
}

impl Dispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn include_router(&mut self, router: Router) -> &mut Self {
        self.routers.push(router);
        self
    }

    /// Adds typed application data available through `UpdateContext::dependency`.
    pub fn provide<T: Any + Send + Sync>(&mut self, value: T) -> &mut Self {
        self.dependencies.insert(TypeId::of::<T>(), Arc::new(value));
        self
    }

    pub fn allowed_updates(
        &mut self,
        updates: impl IntoIterator<Item = impl Into<String>>,
    ) -> &mut Self {
        self.allowed_updates = Some(updates.into_iter().map(Into::into).collect());
        self
    }

    pub fn polling_timeout(&mut self, timeout: Duration) -> &mut Self {
        self.polling_timeout = timeout;
        self
    }

    pub fn backoff_config(&mut self, config: BackoffConfig) -> &mut Self {
        self.backoff_config = config;
        self
    }

    pub fn handle_as_tasks(&mut self, value: bool) -> &mut Self {
        self.handle_as_tasks = value;
        self
    }

    pub fn tasks_concurrency_limit(&mut self, limit: usize) -> Result<&mut Self> {
        if limit == 0 {
            return Err(Error::InvalidPayload(
                "tasks concurrency limit must be greater than zero".to_owned(),
            ));
        }
        self.tasks_concurrency_limit = Some(limit);
        Ok(self)
    }

    /// Installs FSM context and event isolation before filters are evaluated.
    pub fn fsm(&mut self, middleware: FsmMiddleware) -> &mut Self {
        self.fsm_middleware = Some(middleware);
        self
    }

    pub fn fsm_middleware(&self) -> Option<&FsmMiddleware> {
        self.fsm_middleware.as_ref()
    }

    pub fn storage(&self) -> Option<Arc<dyn crate::fsm::Storage>> {
        self.fsm_middleware.as_ref().map(FsmMiddleware::storage)
    }

    pub fn startup<F, Fut>(&mut self, handler: F) -> &mut Self
    where
        F: Fn(Bot) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.startup_handlers
            .push(Arc::new(move |bot| Box::pin(handler(bot))));
        self
    }

    pub fn shutdown<F, Fut>(&mut self, handler: F) -> &mut Self
    where
        F: Fn(Bot) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.shutdown_handlers
            .push(Arc::new(move |bot| Box::pin(handler(bot))));
        self
    }

    pub async fn emit_startup(&self, bot: Bot) -> Result<()> {
        Self::emit_lifecycle(&self.startup_handlers, bot.clone()).await?;
        for router in &self.routers {
            router.emit_lifecycle(bot.clone(), true).await?;
        }
        Ok(())
    }

    pub async fn emit_shutdown(&self, bot: Bot) -> Result<()> {
        let lifecycle_result = async {
            Self::emit_lifecycle(&self.shutdown_handlers, bot.clone()).await?;
            for router in &self.routers {
                router.emit_lifecycle(bot.clone(), false).await?;
            }
            Ok(())
        }
        .await;
        let fsm_result = match &self.fsm_middleware {
            Some(fsm) => fsm.close().await,
            None => Ok(()),
        };
        lifecycle_result.and(fsm_result)
    }

    async fn emit_lifecycle(handlers: &[LifecycleHandler], bot: Bot) -> Result<()> {
        for handler in handlers {
            handler(bot.clone()).await?;
        }
        Ok(())
    }

    /// Computes Telegram update types registered across the complete router tree.
    /// Returns `None` if a catch-all update handler requires unrestricted updates.
    pub fn resolve_used_update_types(&self) -> Option<Vec<String>> {
        let mut updates = BTreeSet::new();
        let mut has_catch_all = false;
        for router in &self.routers {
            has_catch_all |= router.collect_used_update_types(&mut updates);
        }
        (!has_catch_all).then(|| updates.into_iter().collect())
    }

    pub async fn feed_update(&self, bot: Bot, update: Update) -> Result<bool> {
        self.feed_update_inner(bot, update, None).await
    }

    /// Validates and dispatches an update represented as raw JSON data.
    ///
    /// This is the typed Rust counterpart of aiogram's `feed_raw_update` and
    /// is useful for brokers, recorded updates and framework integrations that
    /// have already parsed the HTTP request body.
    pub async fn feed_raw_update(&self, bot: Bot, update: serde_json::Value) -> Result<bool> {
        self.feed_update(bot, serde_json::from_value(update)?).await
    }

    pub async fn feed_webhook_update(
        &self,
        bot: Bot,
        update: Update,
    ) -> Result<(bool, Option<BotRequest>)> {
        let response = Arc::new(Mutex::new(None));
        let handled = self
            .feed_update_inner(bot, update, Some(response.clone()))
            .await?;
        let answer = response
            .lock()
            .map_err(|_| Error::Handler("webhook response lock poisoned".to_owned()))?
            .take();
        Ok((handled, answer))
    }

    async fn feed_update_inner(
        &self,
        bot: Bot,
        update: Update,
        webhook_response: Option<Arc<Mutex<Option<BotRequest>>>>,
    ) -> Result<bool> {
        let mut context = UpdateContext {
            bot,
            update: Arc::new(update),
            dependencies: Dependencies::new(self.dependencies.clone()),
            webhook_response,
            handler_error: None,
        };
        context = context.with_dependency(EventContext::resolve(&context.update));
        let _fsm_guard = if let Some(middleware) = &self.fsm_middleware {
            if let Some((fsm, guard)) = middleware.resolve(&context).await? {
                context = context.with_dependency(fsm);
                Some(guard)
            } else {
                None
            }
        } else {
            None
        };
        for router in &self.routers {
            match router.dispatch(context.clone(), Vec::new()).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(error) => return self.dispatch_error(context, error).await,
            }
        }
        Ok(false)
    }

    async fn dispatch_error(&self, context: UpdateContext, error: Error) -> Result<bool> {
        let error = Arc::new(error);
        let error_context = context.with_error(error.clone());
        for router in &self.routers {
            match router.dispatch(error_context.clone(), Vec::new()).await {
                Ok(true) => return Ok(true),
                Ok(false) => {}
                Err(error) => return Err(error),
            }
        }
        drop(error_context);
        Err(Arc::try_unwrap(error).unwrap_or_else(|error| Error::Handler(error.to_string())))
    }

    /// Starts long polling and shuts down on Ctrl+C/SIGINT.
    pub async fn start_polling(&self, bot: Bot) -> Result<()> {
        self.start_polling_many([bot]).await
    }

    /// Starts one polling worker per bot under one shared lifecycle and stop
    /// signal, matching aiogram's multi-bot polling capability.
    pub async fn start_polling_many(&self, bots: impl IntoIterator<Item = Bot>) -> Result<()> {
        let bots: Vec<_> = bots.into_iter().collect();
        if bots.is_empty() {
            return Err(Error::InvalidPayload(
                "at least one bot is required to start polling".to_owned(),
            ));
        }
        let (shutdown_tx, shutdown_rx) = self.polling_control.start()?;
        let signal_tx = shutdown_tx.clone();
        let signal = tokio::spawn(async move {
            polling_shutdown_signal().await;
            let _ = signal_tx.send(true);
        });
        let result = self.poll_many(bots, shutdown_rx).await;
        signal.abort();
        self.polling_control.finish();
        result
    }

    pub fn is_polling(&self) -> bool {
        self.polling_control.is_running()
    }

    /// Requests graceful shutdown and waits until all polling workers and the
    /// dispatcher shutdown lifecycle have finished.
    pub async fn stop_polling(&self) -> Result<()> {
        self.polling_control.stop().await
    }

    pub async fn wait_stopped(&self) {
        self.polling_control.wait_stopped().await;
    }

    /// Long-polls until `shutdown` becomes true.
    pub async fn poll(&self, bot: Bot, mut shutdown: watch::Receiver<bool>) -> Result<()> {
        self.emit_startup(bot.clone()).await?;
        let result = self.poll_updates(bot.clone(), &mut shutdown).await;
        let shutdown_result = self.emit_shutdown(bot).await;
        result.and(shutdown_result)
    }

    /// Polls several bots until the shared shutdown receiver changes. Startup
    /// and shutdown observers are emitted once, using the final bot as aiogram
    /// does for its lifecycle context.
    pub async fn poll_many(
        &self,
        bots: impl IntoIterator<Item = Bot>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<()> {
        let bots: Vec<_> = bots.into_iter().collect();
        let lifecycle_bot = bots.last().cloned().ok_or_else(|| {
            Error::InvalidPayload("at least one bot is required to poll".to_owned())
        })?;
        self.emit_startup(lifecycle_bot.clone()).await?;
        let mut workers = JoinSet::new();
        for bot in bots {
            let dispatcher = self.clone();
            let mut bot_shutdown = shutdown.clone();
            workers.spawn(async move { dispatcher.poll_updates(bot, &mut bot_shutdown).await });
        }
        let mut result = Ok(());
        while let Some(worker) = workers.join_next().await {
            match worker {
                Ok(Ok(())) => {}
                Ok(Err(error)) if result.is_ok() => result = Err(error),
                Err(error) if result.is_ok() => {
                    result = Err(Error::Handler(format!("polling worker failed: {error}")));
                }
                _ => {}
            }
        }
        let shutdown_result = self.emit_shutdown(lifecycle_bot).await;
        result.and(shutdown_result)
    }

    async fn poll_updates(&self, bot: Bot, shutdown: &mut watch::Receiver<bool>) -> Result<()> {
        let mut offset = None;
        let auto_updates = self.resolve_used_update_types();
        let mut backoff = Backoff::new(self.backoff_config);
        let semaphore = self
            .tasks_concurrency_limit
            .map(|limit| Arc::new(Semaphore::new(limit)));
        let mut tasks = JoinSet::new();
        'polling: loop {
            if *shutdown.borrow() {
                break;
            }
            let request = GetUpdates {
                offset,
                limit: Some(100),
                timeout: Some(self.polling_timeout.as_secs().min(i64::MAX as u64) as i64),
                allowed_updates: self
                    .allowed_updates
                    .clone()
                    .or_else(|| auto_updates.clone()),
                extra: Default::default(),
            };
            let fetched = tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break 'polling; }
                    continue;
                }
                updates = bot.get_updates(request) => updates,
            };
            let updates = match fetched {
                Ok(updates) => {
                    if backoff.counter() > 0 {
                        tracing::info!(attempts = backoff.counter(), "polling connection restored");
                        backoff.reset();
                    }
                    updates
                }
                Err(error) => {
                    let delay = backoff.advance();
                    tracing::error!(%error, ?delay, attempt = backoff.counter(), "failed to fetch updates; retrying");
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() { break 'polling; }
                        }
                    }
                    continue;
                }
            };
            for update in updates {
                offset = Some(update.update_id + 1);
                if self.handle_as_tasks {
                    let permit = if let Some(semaphore) = &semaphore {
                        Some(
                            semaphore
                                .clone()
                                .acquire_owned()
                                .await
                                .map_err(|_| Error::DispatcherStopped)?,
                        )
                    } else {
                        None
                    };
                    let dispatcher = self.clone();
                    let bot = bot.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        if let Err(error) = dispatcher.feed_update(bot, update).await {
                            tracing::error!(%error, "update handler failed");
                        }
                    });
                } else if let Err(error) = self.feed_update(bot.clone(), update).await {
                    tracing::error!(%error, "update handler failed");
                }
            }
            while let Some(result) = tasks.try_join_next() {
                if let Err(error) = result {
                    tracing::error!(%error, "update task panicked");
                }
            }
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::error!(%error, "update task panicked during shutdown");
            }
        }
        Ok(())
    }
}

/// Continue looking for another handler after the current one matched.
pub fn skip() -> Result<()> {
    Err(Error::SkipHandler)
}

/// Stop propagation and treat the update as handled.
pub fn cancel() -> Result<()> {
    Err(Error::CancelHandler)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::filters;
    use crate::filters::FilterExt;

    fn update(field: &str, text: &str) -> Update {
        serde_json::from_value(serde_json::json!({
            "update_id": 1,
            (field): {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 4, "type": "private"},
                "from": {"id": 4, "is_bot": false, "first_name": "Ada"},
                "text": text
            }
        }))
        .unwrap()
    }

    fn bot() -> Bot {
        Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap()
    }

    #[tokio::test]
    async fn routes_exact_event_and_injects_dependency() {
        let mut router = Router::new();
        router.message(filters::command("start"), |context| async move {
            context
                .dependency::<AtomicUsize>()
                .unwrap()
                .fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .include_router(router)
            .provide(AtomicUsize::new(0));

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/start payload"))
                .await
                .unwrap()
        );
        assert!(
            !dispatcher
                .feed_update(bot(), update("edited_message", "/start payload"))
                .await
                .unwrap()
        );
        assert_eq!(
            dispatcher
                .dependencies
                .get(&TypeId::of::<AtomicUsize>())
                .unwrap()
                .clone()
                .downcast::<AtomicUsize>()
                .unwrap()
                .load(Ordering::SeqCst),
            1
        );
    }

    #[tokio::test]
    async fn feed_raw_update_validates_and_dispatches_json_values() {
        let seen = Arc::new(AtomicBool::new(false));
        let mut router = Router::new();
        let handler_seen = seen.clone();
        router.message(filters::any(), move |_| {
            let handler_seen = handler_seen.clone();
            async move {
                handler_seen.store(true, Ordering::SeqCst);
                Ok(())
            }
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        let raw = serde_json::to_value(update("message", "hello")).unwrap();
        assert!(dispatcher.feed_raw_update(bot(), raw).await.unwrap());
        assert!(seen.load(Ordering::SeqCst));

        let invalid = serde_json::json!({"update_id": "not-an-integer"});
        assert!(matches!(
            dispatcher.feed_raw_update(bot(), invalid).await,
            Err(Error::Json(_))
        ));
    }

    #[tokio::test]
    async fn nested_router_and_multiple_filters_propagate() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mut child = Router::named("child");
        child.message_filters(
            vec![
                filters::boxed(filters::command("start")),
                filters::boxed(filters::text("/start")),
            ],
            move |_| {
                let captured = captured.clone();
                async move {
                    captured.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let mut root = Router::named("root");
        root.include_router(child);
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(root);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/start"))
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            dispatcher.resolve_used_update_types(),
            Some(vec!["message".to_owned()])
        );
    }

    #[tokio::test]
    async fn typed_handler_flags_reach_filters_middleware_and_handler() {
        struct FlagMiddleware;

        #[async_trait]
        impl Middleware for FlagMiddleware {
            async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
                assert_eq!(*context.handler_flag::<u32>("rate_limit").unwrap(), 7);
                assert_eq!(
                    context
                        .handler_flags()
                        .unwrap()
                        .get::<Vec<filters::Command>>("commands")
                        .unwrap()
                        .len(),
                    1
                );
                next.run(context).await
            }
        }

        let flags = HandlerFlags::new().with("rate_limit", 7_u32);
        let mut router = Router::new();
        router.middleware(FlagMiddleware);
        router.message_filters_with_flags(
            vec![
                filters::boxed(filters::FnFilter::new(|context| {
                    Box::pin(async move {
                        context
                            .handler_flag::<u32>("rate_limit")
                            .is_some_and(|value| *value == 7)
                    })
                })),
                filters::boxed(filters::Command::new("start")),
            ],
            flags,
            |context| async move {
                assert_eq!(*context.handler_flag::<u32>("rate_limit").unwrap(), 7);
                Ok(())
            },
        );
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/start"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn command_filter_validates_mentions_and_arguments() {
        let mut router = Router::new();
        router.message(
            filters::Command::new("start").bot_username("right_bot"),
            |context| async move {
                let command = context.dependency::<filters::CommandMatch>().unwrap();
                assert_eq!(command.command, "start");
                assert_eq!(command.mention.as_deref(), Some("right_bot"));
                assert_eq!(command.args.as_deref(), Some("payload"));
                Ok(())
            },
        );
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            !dispatcher
                .feed_update(bot(), update("message", "/start@wrong_bot payload"))
                .await
                .unwrap()
        );
        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/start@right_bot payload"))
                .await
                .unwrap()
        );
        assert_eq!(
            filters::parse_command("/start one two").unwrap().args,
            Some("one two")
        );
    }

    #[tokio::test]
    async fn command_start_decodes_payload_and_regex_captures_are_injected() {
        let mut router = Router::new();
        router.message(
            filters::CommandStart::new().deep_link(true).encoded(true),
            |context| async move {
                assert_eq!(
                    context
                        .dependency::<filters::CommandMatch>()
                        .unwrap()
                        .args
                        .as_deref(),
                    Some("hello world")
                );
                Ok(())
            },
        );
        router.message(
            filters::Command::regex(r"^item_(\d+)$").unwrap(),
            |context| async move {
                let command = context.dependency::<filters::CommandMatch>().unwrap();
                assert_eq!(
                    command.regex_captures.as_ref().unwrap()[1].as_deref(),
                    Some("42")
                );
                Ok(())
            },
        );
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/start aGVsbG8gd29ybGQ"))
                .await
                .unwrap()
        );
        assert!(
            dispatcher
                .feed_update(bot(), update("message", "/item_42"))
                .await
                .unwrap()
        );
        assert!(
            !dispatcher
                .feed_update(bot(), update("message", "/START aGVsbG8gd29ybGQ"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn callback_filter_injects_parsed_typed_data() {
        let mut router = Router::new();
        router.callback_query(
            filters::callback_data_filter(
                |value| {
                    value
                        .strip_prefix("item:")
                        .ok_or_else(|| Error::Utility("prefix".to_owned()))?
                        .parse::<u64>()
                        .map_err(|error| Error::Utility(error.to_string()))
                },
                |value| *value > 0,
            ),
            |context| async move {
                assert_eq!(*context.dependency::<u64>().unwrap(), 42);
                Ok(())
            },
        );
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let update = serde_json::from_value(serde_json::json!({
            "update_id": 1,
            "callback_query": {
                "id": "query",
                "from": {"id": 1, "is_bot": false, "first_name": "Ada"},
                "chat_instance": "instance",
                "data": "item:42"
            }
        }))
        .unwrap();

        assert!(dispatcher.feed_update(bot(), update).await.unwrap());
    }

    #[tokio::test]
    async fn magic_fields_match_nested_values_and_capture_data() {
        let mut router = Router::new();
        router.message_filters(
            vec![
                filters::boxed(filters::field("text").regex(r"^hello-\d+$").unwrap()),
                filters::boxed(filters::field("chat.id").equals(4_i64)),
                filters::boxed(filters::update_field("message.text").capture::<String>()),
            ],
            |context| async move {
                assert_eq!(context.dependency::<String>().unwrap().as_str(), "hello-42");
                Ok(())
            },
        );
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "hello-42"))
                .await
                .unwrap()
        );
        assert!(
            !dispatcher
                .feed_update(bot(), update("message", "goodbye"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn fsm_context_is_available_to_state_filters_and_handlers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let state = crate::fsm::State::new("Form", "name");
        let next_state = state.clone();
        let mut router = Router::new();
        router.message(filters::command("begin"), move |context| {
            let next_state = next_state.clone();
            async move {
                context
                    .dependency::<crate::fsm::FsmContext>()
                    .unwrap()
                    .set_state(next_state)
                    .await
            }
        });
        router.message(
            filters::StateFilter::new(state).and(filters::text("next")),
            move |_| {
                let captured = captured.clone();
                async move {
                    captured.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let storage = Arc::new(crate::fsm::MemoryStorage::default());
        let mut dispatcher = Dispatcher::new();
        dispatcher
            .include_router(router)
            .fsm(crate::fsm::FsmMiddleware::new(storage));

        dispatcher
            .feed_update(bot(), update("message", "/begin"))
            .await
            .unwrap();
        dispatcher
            .feed_update(bot(), update("message", "next"))
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    struct TraceMiddleware(Arc<Mutex<Vec<&'static str>>>);

    #[async_trait]
    impl Middleware for TraceMiddleware {
        async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
            self.0.lock().unwrap().push("before");
            next.run(context).await?;
            self.0.lock().unwrap().push("after");
            Ok(())
        }
    }

    struct TraceOuterMiddleware(Arc<Mutex<Vec<&'static str>>>);

    #[async_trait]
    impl OuterMiddleware for TraceOuterMiddleware {
        async fn handle(&self, context: UpdateContext, next: OuterNext<'_>) -> Result<bool> {
            self.0.lock().unwrap().push("outer-before");
            let handled = next.run(context).await?;
            self.0.lock().unwrap().push("outer-after");
            Ok(handled)
        }
    }

    #[tokio::test]
    async fn middleware_wraps_handler_and_skip_continues() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let first_trace = trace.clone();
        let second_trace = trace.clone();
        let mut router = Router::new();
        router.middleware(TraceMiddleware(trace.clone()));
        router.message(filters::any(), move |_| {
            let trace = first_trace.clone();
            async move {
                trace.lock().unwrap().push("skip");
                skip()
            }
        });
        router.message(filters::any(), move |_| {
            let trace = second_trace.clone();
            async move {
                trace.lock().unwrap().push("handler");
                Ok(())
            }
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "hello"))
                .await
                .unwrap()
        );
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["before", "skip", "before", "handler", "after"]
        );
    }

    #[tokio::test]
    async fn outer_middleware_runs_before_filters_even_when_no_route_matches() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut router = Router::new();
        router.outer_middleware(TraceOuterMiddleware(trace.clone()));
        router.message(filters::text("expected"), |_| async { Ok(()) });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            !dispatcher
                .feed_update(bot(), update("message", "different"))
                .await
                .unwrap()
        );
        assert_eq!(*trace.lock().unwrap(), vec!["outer-before", "outer-after"]);
    }

    #[tokio::test]
    async fn observer_root_filter_and_middlewares_scope_to_event_and_gate_children() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let child_calls = calls.clone();
        let mut child = Router::new();
        child.message(filters::any(), move |_| {
            let child_calls = child_calls.clone();
            async move {
                child_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        child.edited_message(filters::any(), |_| async { Ok(()) });

        let mut root = Router::new();
        root.message_filter(filters::text("allowed"));
        root.message_middleware(TraceMiddleware(trace.clone()));
        root.message_outer_middleware(TraceOuterMiddleware(trace.clone()));
        root.include_router(child);
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(root);

        assert!(
            !dispatcher
                .feed_update(bot(), update("message", "blocked"))
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(*trace.lock().unwrap(), vec!["outer-before", "outer-after"]);
        trace.lock().unwrap().clear();

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "allowed"))
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *trace.lock().unwrap(),
            vec!["outer-before", "before", "after", "outer-after"]
        );
        trace.lock().unwrap().clear();

        assert!(
            dispatcher
                .feed_update(bot(), update("edited_message", "blocked"))
                .await
                .unwrap()
        );
        assert!(trace.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn error_observer_receives_handler_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mut router = Router::new();
        router.message(filters::any(), |_| async {
            Err(Error::Handler("boom".to_owned()))
        });
        router.error(filters::any(), move |context| {
            let captured = captured.clone();
            async move {
                assert_eq!(context.event_type(), Some("error"));
                assert_eq!(context.error().unwrap().to_string(), "handler error: boom");
                captured.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(
            dispatcher
                .feed_update(bot(), update("message", "hello"))
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            dispatcher.resolve_used_update_types(),
            Some(vec!["message".to_owned()])
        );
    }

    #[tokio::test]
    async fn unhandled_error_is_returned_to_caller() {
        let mut router = Router::new();
        router.message(filters::any(), |_| async {
            Err(Error::Handler("unhandled".to_owned()))
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);

        assert!(matches!(
            dispatcher
                .feed_update(bot(), update("message", "hello"))
                .await,
            Err(Error::Handler(message)) if message == "unhandled"
        ));
    }

    struct CountingClassHandler(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl ClassHandler for CountingClassHandler {
        async fn handle(&self, context: UpdateContext) -> Result<()> {
            assert_eq!(context.message().unwrap().text.as_deref(), Some("hello"));
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn class_handler_trait_registers_stateful_handlers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut router = Router::new();
        router.message_handler(filters::any(), CountingClassHandler(calls.clone()));
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        assert!(
            dispatcher
                .feed_update(bot(), update("message", "hello"))
                .await
                .unwrap()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn multi_bot_polling_has_one_lifecycle_and_programmatic_shutdown() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_base = format!("http://{}", listener.local_addr().unwrap());
        let startup_calls = Arc::new(AtomicUsize::new(0));
        let shutdown_calls = Arc::new(AtomicUsize::new(0));
        let mut dispatcher = Dispatcher::new();
        let calls = startup_calls.clone();
        dispatcher.startup(move |bot| {
            let calls = calls.clone();
            async move {
                assert_eq!(bot.id(), 654321);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let calls = shutdown_calls.clone();
        dispatcher.shutdown(move |bot| {
            let calls = calls.clone();
            async move {
                assert_eq!(bot.id(), 654321);
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let dispatcher = Arc::new(dispatcher);
        let polling_dispatcher = dispatcher.clone();
        let first =
            Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base.clone()).unwrap();
        let second =
            Bot::with_api_base("654321:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        let polling =
            tokio::spawn(
                async move { polling_dispatcher.start_polling_many([first, second]).await },
            );

        tokio::time::timeout(Duration::from_secs(2), async {
            while !dispatcher.is_polling() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(matches!(
            dispatcher.start_polling(bot()).await.unwrap_err(),
            Error::PollingAlreadyStarted
        ));
        tokio::time::timeout(Duration::from_secs(2), dispatcher.stop_polling())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), polling)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!dispatcher.is_polling());
        assert_eq!(startup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(shutdown_calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            dispatcher.stop_polling().await.unwrap_err(),
            Error::PollingNotStarted
        ));
        drop(listener);
    }

    #[tokio::test]
    async fn lifecycle_hooks_run_in_registration_order() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let startup_trace = trace.clone();
        let shutdown_trace = trace.clone();
        let router_startup_trace = trace.clone();
        let router_shutdown_trace = trace.clone();
        let child_startup_trace = trace.clone();
        let child_shutdown_trace = trace.clone();
        let mut child = Router::new();
        child.startup(move |_| {
            let trace = child_startup_trace.clone();
            async move {
                trace.lock().unwrap().push("child-startup");
                Ok(())
            }
        });
        child.shutdown(move |_| {
            let trace = child_shutdown_trace.clone();
            async move {
                trace.lock().unwrap().push("child-shutdown");
                Ok(())
            }
        });
        let mut router = Router::new();
        router.startup(move |_| {
            let trace = router_startup_trace.clone();
            async move {
                trace.lock().unwrap().push("router-startup");
                Ok(())
            }
        });
        router.shutdown(move |_| {
            let trace = router_shutdown_trace.clone();
            async move {
                trace.lock().unwrap().push("router-shutdown");
                Ok(())
            }
        });
        router.include_router(child);
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        dispatcher.startup(move |_| {
            let trace = startup_trace.clone();
            async move {
                trace.lock().unwrap().push("startup");
                Ok(())
            }
        });
        dispatcher.shutdown(move |_| {
            let trace = shutdown_trace.clone();
            async move {
                trace.lock().unwrap().push("shutdown");
                Ok(())
            }
        });

        dispatcher.emit_startup(bot()).await.unwrap();
        dispatcher.emit_shutdown(bot()).await.unwrap();
        assert_eq!(
            *trace.lock().unwrap(),
            vec![
                "startup",
                "router-startup",
                "child-startup",
                "shutdown",
                "router-shutdown",
                "child-shutdown"
            ]
        );
    }
}
