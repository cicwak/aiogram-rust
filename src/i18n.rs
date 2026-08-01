//! Lightweight internationalization catalogs and dispatcher middleware.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::dispatcher::{Middleware, Next, UpdateContext};
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
enum Translation {
    Singular(String),
    Plural { one: String, other: String },
}

/// Thread-safe translation registry with default-locale fallback.
#[derive(Debug, Clone)]
pub struct I18n {
    default_locale: Arc<str>,
    catalogs: Arc<RwLock<BTreeMap<String, BTreeMap<String, Translation>>>>,
    gettext_catalogs: Arc<RwLock<BTreeMap<String, gettext::Catalog>>>,
    catalog_source: Arc<RwLock<Option<CatalogSource>>>,
}

#[derive(Debug, Clone)]
struct CatalogSource {
    path: PathBuf,
    domain: String,
}

impl I18n {
    pub fn new(default_locale: impl Into<String>) -> Self {
        Self {
            default_locale: Arc::from(default_locale.into()),
            catalogs: Arc::new(RwLock::new(BTreeMap::new())),
            gettext_catalogs: Arc::new(RwLock::new(BTreeMap::new())),
            catalog_source: Arc::new(RwLock::new(None)),
        }
    }

    /// Loads GNU gettext catalogs from aiogram's conventional directory tree:
    /// `<path>/<locale>/LC_MESSAGES/<domain>.mo`.
    pub fn from_path(
        path: impl AsRef<Path>,
        default_locale: impl Into<String>,
        domain: impl Into<String>,
    ) -> Result<Self> {
        let i18n = Self::new(default_locale);
        *i18n
            .catalog_source
            .write()
            .map_err(|_| Error::Utility("i18n source lock poisoned".to_owned()))? =
            Some(CatalogSource {
                path: path.as_ref().to_path_buf(),
                domain: domain.into(),
            });
        i18n.reload()?;
        Ok(i18n)
    }

    pub fn default_locale(&self) -> &str {
        &self.default_locale
    }

    pub fn available_locales(&self) -> Vec<String> {
        let mut locales = self
            .catalogs
            .read()
            .map(|catalogs| catalogs.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(catalogs) = self.gettext_catalogs.read() {
            locales.extend(catalogs.keys().cloned());
        }
        locales.sort();
        locales.dedup();
        locales
    }

    pub fn has_locale(&self, locale: &str) -> bool {
        self.catalogs
            .read()
            .is_ok_and(|catalogs| catalogs.contains_key(locale))
            || self
                .gettext_catalogs
                .read()
                .is_ok_and(|catalogs| catalogs.contains_key(locale))
    }

    fn normalize_locale(&self, locale: Option<&str>) -> String {
        if let Some(locale) = locale {
            if self.has_locale(locale) {
                return locale.to_owned();
            }
            let normalized = locale.replace('-', "_");
            if self.has_locale(&normalized) {
                return normalized;
            }
            if let Some(language) = normalized.split('_').next()
                && self.has_locale(language)
            {
                return language.to_owned();
            }
        }
        self.default_locale().to_owned()
    }

    /// Adds a compiled GNU MO catalog from any reader.
    pub fn add_gettext_catalog(&self, locale: impl Into<String>, reader: impl Read) -> Result<()> {
        let catalog = gettext::Catalog::parse(reader)
            .map_err(|error| Error::Utility(format!("invalid gettext catalog: {error}")))?;
        self.gettext_catalogs
            .write()
            .map_err(|_| Error::Utility("gettext catalog lock poisoned".to_owned()))?
            .insert(locale.into(), catalog);
        Ok(())
    }

    /// Reloads every MO catalog previously configured with [`I18n::from_path`].
    pub fn reload(&self) -> Result<()> {
        let source = self
            .catalog_source
            .read()
            .map_err(|_| Error::Utility("i18n source lock poisoned".to_owned()))?
            .clone()
            .ok_or_else(|| Error::Utility("reload() requires I18n::from_path()".to_owned()))?;
        let mut catalogs = BTreeMap::new();
        let entries = std::fs::read_dir(&source.path).map_err(|error| {
            Error::Utility(format!(
                "cannot read locale directory {}: {error}",
                source.path.display()
            ))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                Error::Utility(format!("cannot read locale directory entry: {error}"))
            })?;
            let file_type = entry.file_type().map_err(|error| {
                Error::Utility(format!("cannot inspect locale directory entry: {error}"))
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let locale = entry.file_name().to_string_lossy().into_owned();
            let base = entry.path().join("LC_MESSAGES").join(&source.domain);
            let mo_path = base.with_extension("mo");
            if mo_path.exists() {
                let file = File::open(&mo_path).map_err(|error| {
                    Error::Utility(format!(
                        "cannot open gettext catalog {}: {error}",
                        mo_path.display()
                    ))
                })?;
                let catalog = gettext::Catalog::parse(file).map_err(|error| {
                    Error::Utility(format!(
                        "invalid gettext catalog {}: {error}",
                        mo_path.display()
                    ))
                })?;
                catalogs.insert(locale, catalog);
            } else {
                let po_path = base.with_extension("po");
                if po_path.exists() {
                    return Err(Error::Utility(format!(
                        "locale {locale:?} has an uncompiled PO catalog at {}",
                        po_path.display()
                    )));
                }
            }
        }
        *self
            .gettext_catalogs
            .write()
            .map_err(|_| Error::Utility("gettext catalog lock poisoned".to_owned()))? = catalogs;
        Ok(())
    }

    pub fn add(
        &self,
        locale: impl Into<String>,
        message_id: impl Into<String>,
        translation: impl Into<String>,
    ) -> Result<()> {
        self.catalogs
            .write()
            .map_err(|_| Error::Utility("i18n catalog lock poisoned".to_owned()))?
            .entry(locale.into())
            .or_default()
            .insert(message_id.into(), Translation::Singular(translation.into()));
        Ok(())
    }

    pub fn add_plural(
        &self,
        locale: impl Into<String>,
        message_id: impl Into<String>,
        one: impl Into<String>,
        other: impl Into<String>,
    ) -> Result<()> {
        self.catalogs
            .write()
            .map_err(|_| Error::Utility("i18n catalog lock poisoned".to_owned()))?
            .entry(locale.into())
            .or_default()
            .insert(
                message_id.into(),
                Translation::Plural {
                    one: one.into(),
                    other: other.into(),
                },
            );
        Ok(())
    }

    pub fn gettext(
        &self,
        locale: &str,
        message_id: &str,
        variables: &BTreeMap<String, String>,
    ) -> String {
        let locale = self.normalize_locale(Some(locale));
        self.translate(&locale, message_id, None, None, variables)
    }

    pub fn ngettext(
        &self,
        locale: &str,
        message_id: &str,
        count: i64,
        variables: &BTreeMap<String, String>,
    ) -> String {
        let locale = self.normalize_locale(Some(locale));
        let mut variables = variables.clone();
        variables
            .entry("n".to_owned())
            .or_insert_with(|| count.to_string());
        self.translate(
            &locale,
            message_id,
            Some(message_id),
            Some(count),
            &variables,
        )
    }

    /// Translates a singular/plural source pair using the locale's GNU plural
    /// expression. This supports languages with more than two plural forms.
    pub fn ngettext_with_plural(
        &self,
        locale: &str,
        singular: &str,
        plural: &str,
        count: i64,
        variables: &BTreeMap<String, String>,
    ) -> String {
        let locale = self.normalize_locale(Some(locale));
        let mut variables = variables.clone();
        variables
            .entry("n".to_owned())
            .or_insert_with(|| count.to_string());
        self.translate(&locale, singular, Some(plural), Some(count), &variables)
    }

    fn translate(
        &self,
        locale: &str,
        message_id: &str,
        plural: Option<&str>,
        count: Option<i64>,
        variables: &BTreeMap<String, String>,
    ) -> String {
        let Ok(catalogs) = self.catalogs.read() else {
            return format_variables(source_form(message_id, plural, count), variables);
        };
        let translation = catalogs
            .get(locale)
            .and_then(|catalog| catalog.get(message_id))
            .or_else(|| {
                catalogs
                    .get(self.default_locale())
                    .and_then(|catalog| catalog.get(message_id))
            });
        let text = match translation {
            Some(Translation::Singular(value)) => value,
            Some(Translation::Plural { one, other }) if count == Some(1) => one,
            Some(Translation::Plural { other, .. }) => other,
            None => {
                drop(catalogs);
                return format_variables(
                    &self
                        .gettext_translation(locale, message_id, plural, count)
                        .or_else(|| {
                            self.gettext_translation(
                                self.default_locale(),
                                message_id,
                                plural,
                                count,
                            )
                        })
                        .unwrap_or_else(|| source_form(message_id, plural, count).to_owned()),
                    variables,
                );
            }
        };
        format_variables(text, variables)
    }

    fn gettext_translation(
        &self,
        locale: &str,
        singular: &str,
        plural: Option<&str>,
        count: Option<i64>,
    ) -> Option<String> {
        let catalogs = self.gettext_catalogs.read().ok()?;
        let catalog = catalogs.get(locale)?;
        let translated = match (plural, count) {
            (Some(plural), Some(count)) => catalog.ngettext(singular, plural, count.max(0) as u64),
            _ => catalog.gettext(singular),
        };
        let fallback = source_form(singular, plural, count);
        (translated != fallback).then(|| translated.to_owned())
    }
}

fn source_form<'a>(singular: &'a str, plural: Option<&'a str>, count: Option<i64>) -> &'a str {
    if count.is_some_and(|count| count != 1) {
        plural.unwrap_or(singular)
    } else {
        singular
    }
}

/// Locale-bound translations injected into handlers by `I18nMiddleware`.
#[derive(Debug, Clone)]
pub struct I18nContext {
    i18n: I18n,
    locale: String,
}

impl I18nContext {
    fn new(i18n: I18n, locale: String) -> Self {
        Self { i18n, locale }
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn gettext(&self, message_id: &str, variables: &BTreeMap<String, String>) -> String {
        self.i18n.gettext(&self.locale, message_id, variables)
    }

    pub fn ngettext(
        &self,
        message_id: &str,
        count: i64,
        variables: &BTreeMap<String, String>,
    ) -> String {
        self.i18n
            .ngettext(&self.locale, message_id, count, variables)
    }

    pub fn ngettext_with_plural(
        &self,
        singular: &str,
        plural: &str,
        count: i64,
        variables: &BTreeMap<String, String>,
    ) -> String {
        self.i18n
            .ngettext_with_plural(&self.locale, singular, plural, count, variables)
    }

    pub fn lazy_gettext(&self, message_id: impl Into<String>) -> LazyTranslation {
        LazyTranslation {
            context: self.clone(),
            message_id: message_id.into(),
            plural: None,
            count: None,
            variables: BTreeMap::new(),
        }
    }

    pub fn lazy_ngettext(
        &self,
        singular: impl Into<String>,
        plural: impl Into<String>,
        count: i64,
    ) -> LazyTranslation {
        LazyTranslation {
            context: self.clone(),
            message_id: singular.into(),
            plural: Some(plural.into()),
            count: Some(count),
            variables: BTreeMap::new(),
        }
    }
}

/// Locale-bound translation evaluated only when rendered or converted.
#[derive(Debug, Clone)]
pub struct LazyTranslation {
    context: I18nContext,
    message_id: String,
    plural: Option<String>,
    count: Option<i64>,
    variables: BTreeMap<String, String>,
}

impl LazyTranslation {
    pub fn variable(mut self, name: impl Into<String>, value: impl ToString) -> Self {
        self.variables.insert(name.into(), value.to_string());
        self
    }

    pub fn variables(mut self, values: BTreeMap<String, String>) -> Self {
        self.variables.extend(values);
        self
    }

    pub fn render(&self) -> String {
        match (&self.plural, self.count) {
            (Some(plural), Some(count)) => {
                self.context
                    .ngettext_with_plural(&self.message_id, plural, count, &self.variables)
            }
            _ => self.context.gettext(&self.message_id, &self.variables),
        }
    }
}

impl fmt::Display for LazyTranslation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

impl From<LazyTranslation> for String {
    fn from(value: LazyTranslation) -> Self {
        value.render()
    }
}

#[derive(Debug, Clone)]
pub struct I18nMiddleware {
    i18n: I18n,
}

impl I18nMiddleware {
    pub fn new(i18n: I18n) -> Self {
        Self { i18n }
    }

    fn locale(&self, context: &UpdateContext) -> String {
        let locale = context
            .event_context()
            .and_then(|event| event.user.clone())
            .and_then(|user| user.language_code.clone());
        self.i18n.normalize_locale(locale.as_deref())
    }
}

#[async_trait]
impl Middleware for I18nMiddleware {
    async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
        let locale = self.locale(&context);
        let i18n_context = I18nContext::new(self.i18n.clone(), locale);
        next.run(context.with_dependency(i18n_context)).await
    }
}

#[derive(Debug, Clone)]
pub struct ConstI18nMiddleware {
    i18n: I18n,
    locale: String,
}

impl ConstI18nMiddleware {
    pub fn new(i18n: I18n, locale: impl Into<String>) -> Self {
        Self {
            i18n,
            locale: locale.into(),
        }
    }
}

#[async_trait]
impl Middleware for ConstI18nMiddleware {
    async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
        let locale = self.i18n.normalize_locale(Some(&self.locale));
        next.run(context.with_dependency(I18nContext::new(self.i18n.clone(), locale)))
            .await
    }
}

/// Resolves locale from FSM data and persists a user's detected locale on the
/// first update, matching aiogram's `FSMI18nMiddleware` behavior.
#[derive(Debug, Clone)]
pub struct FsmI18nMiddleware {
    i18n: I18n,
    key: String,
}

impl FsmI18nMiddleware {
    pub fn new(i18n: I18n) -> Self {
        Self {
            i18n,
            key: "locale".to_owned(),
        }
    }

    pub fn key(mut self, value: impl Into<String>) -> Self {
        self.key = value.into();
        self
    }

    pub async fn set_locale(&self, state: &crate::fsm::FsmContext, locale: &str) -> Result<()> {
        state
            .update_data(BTreeMap::from([(
                self.key.clone(),
                serde_json::Value::String(locale.to_owned()),
            )]))
            .await?;
        Ok(())
    }
}

#[async_trait]
impl Middleware for FsmI18nMiddleware {
    async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
        let state = context.dependency::<crate::fsm::FsmContext>();
        let stored = match &state {
            Some(state) => state
                .get_value(&self.key)
                .await?
                .and_then(|value| value.as_str().map(str::to_owned)),
            None => None,
        };
        let detected = context
            .event_context()
            .and_then(|event| event.user.clone())
            .and_then(|user| user.language_code);
        let locale = self
            .i18n
            .normalize_locale(stored.as_deref().or(detected.as_deref()));
        if stored.is_none()
            && let Some(state) = state
        {
            self.set_locale(&state, &locale).await?;
        }
        next.run(context.with_dependency(I18nContext::new(self.i18n.clone(), locale)))
            .await
    }
}

fn format_variables(template: &str, variables: &BTreeMap<String, String>) -> String {
    variables
        .iter()
        .fold(template.to_owned(), |text, (key, value)| {
            text.replace(&format!("{{{key}}}"), value)
        })
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn russian_mo_catalog() -> Vec<u8> {
        let metadata = concat!(
            "Content-Type: text/plain; charset=UTF-8\n",
            "Plural-Forms: nplurals=3; ",
            "plural=((n%10==1 && n%100!=11) ? 0 : ",
            "((n%10>=2 && n%10<=4 && (n%100<12 || n%100>14)) ? 1 : 2));\n",
        );
        let originals = [b"".as_slice(), b"apple\0apples".as_slice()];
        let translations = [metadata.as_bytes(), "яблоко\0яблока\0яблок".as_bytes()];
        let count = originals.len();
        let original_table = 28usize;
        let translation_table = original_table + count * 8;
        let data_start = translation_table + count * 8;
        let mut bytes = vec![0; data_start];
        put_u32(&mut bytes, 0, 0x9504_12de);
        put_u32(&mut bytes, 4, 0);
        put_u32(&mut bytes, 8, count as u32);
        put_u32(&mut bytes, 12, original_table as u32);
        put_u32(&mut bytes, 16, translation_table as u32);

        for (index, value) in originals.iter().enumerate() {
            let offset = bytes.len();
            put_u32(&mut bytes, original_table + index * 8, value.len() as u32);
            put_u32(&mut bytes, original_table + index * 8 + 4, offset as u32);
            bytes.extend_from_slice(value);
            bytes.push(0);
        }
        for (index, value) in translations.iter().enumerate() {
            let offset = bytes.len();
            put_u32(
                &mut bytes,
                translation_table + index * 8,
                value.len() as u32,
            );
            put_u32(&mut bytes, translation_table + index * 8 + 4, offset as u32);
            bytes.extend_from_slice(value);
            bytes.push(0);
        }
        bytes
    }

    #[test]
    fn translates_with_fallback_variables_and_plural() {
        let i18n = I18n::new("en");
        i18n.add("en", "hello", "Hello, {name}!").unwrap();
        i18n.add_plural("en", "items", "{n} item", "{n} items")
            .unwrap();
        let variables = BTreeMap::from([("name".to_owned(), "Ada".to_owned())]);
        assert_eq!(i18n.gettext("en", "hello", &variables), "Hello, Ada!");
        assert_eq!(i18n.gettext("de", "hello", &variables), "Hello, Ada!");
        assert_eq!(i18n.normalize_locale(Some("en-US")), "en");
        assert_eq!(i18n.available_locales(), vec!["en".to_owned()]);
        assert_eq!(i18n.ngettext("en", "items", 1, &BTreeMap::new()), "1 item");
        assert_eq!(i18n.ngettext("en", "items", 3, &BTreeMap::new()), "3 items");

        let context = I18nContext::new(i18n.clone(), "en".to_owned());
        assert_eq!(
            context
                .lazy_gettext("hello")
                .variable("name", "Grace")
                .to_string(),
            "Hello, Grace!"
        );
        assert_eq!(
            context.lazy_ngettext("items", "items", 2).to_string(),
            "2 items"
        );
    }

    #[test]
    fn loads_aiogram_style_mo_catalog_and_uses_gnu_plural_rule() {
        let root = std::env::temp_dir().join(format!(
            "aiogram-i18n-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let locale_dir = root.join("ru").join("LC_MESSAGES");
        std::fs::create_dir_all(&locale_dir).unwrap();
        std::fs::write(locale_dir.join("messages.mo"), russian_mo_catalog()).unwrap();

        let i18n = I18n::from_path(&root, "en", "messages").unwrap();
        let variables = BTreeMap::new();
        assert_eq!(i18n.available_locales(), vec!["ru".to_owned()]);
        assert_eq!(
            i18n.ngettext_with_plural("ru-RU", "apple", "apples", 1, &variables),
            "яблоко"
        );
        assert_eq!(
            i18n.ngettext_with_plural("ru", "apple", "apples", 2, &variables),
            "яблока"
        );
        assert_eq!(
            i18n.ngettext_with_plural("ru", "apple", "apples", 5, &variables),
            "яблок"
        );
        assert_eq!(
            i18n.ngettext_with_plural("ru", "apple", "apples", 21, &variables),
            "яблоко"
        );
        assert_eq!(
            i18n.ngettext_with_plural("de", "apple", "apples", 2, &variables),
            "apples"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
