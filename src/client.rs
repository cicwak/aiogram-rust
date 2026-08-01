//! Bot client configuration shared by all typed Telegram methods.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::bot::Bot;
use crate::error::Result;
use crate::types::{InputFileUpload, LinkPreviewOptions};

/// Configurable Bot API endpoints, including Telegram's test environment and
/// local Bot API server file-path mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramApiServer {
    pub api: String,
    pub file: String,
    pub is_local: bool,
    local_paths: Option<(PathBuf, PathBuf)>,
}

impl TelegramApiServer {
    pub fn production() -> Self {
        Self {
            api: "https://api.telegram.org/bot{token}/{method}".to_owned(),
            file: "https://api.telegram.org/file/bot{token}/{path}".to_owned(),
            is_local: false,
            local_paths: None,
        }
    }

    pub fn test() -> Self {
        Self {
            api: "https://api.telegram.org/bot{token}/test/{method}".to_owned(),
            file: "https://api.telegram.org/file/bot{token}/test/{path}".to_owned(),
            is_local: false,
            local_paths: None,
        }
    }

    pub fn from_base(base: impl AsRef<str>) -> Self {
        let base = base.as_ref().trim_end_matches('/');
        Self {
            api: format!("{base}/bot{{token}}/{{method}}"),
            file: format!("{base}/file/bot{{token}}/{{path}}"),
            is_local: false,
            local_paths: None,
        }
    }

    pub fn local(mut self, value: bool) -> Self {
        self.is_local = value;
        self
    }

    /// Maps paths returned by a local Bot API server into paths visible to the
    /// bot process, equivalent to aiogram's `SimpleFilesPathWrapper`.
    pub fn local_file_paths(
        mut self,
        server_path: impl Into<PathBuf>,
        local_path: impl Into<PathBuf>,
    ) -> Self {
        self.local_paths = Some((server_path.into(), local_path.into()));
        self.is_local = true;
        self
    }

    pub fn api_url(&self, token: &str, method: &str) -> String {
        self.api
            .replace("{token}", token)
            .replace("{method}", method)
    }

    pub fn file_url(&self, token: &str, path: &str) -> String {
        self.file
            .replace("{token}", token)
            .replace("{path}", path.trim_start_matches('/'))
    }

    pub fn to_local(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        let Some((server_path, local_path)) = &self.local_paths else {
            return Ok(path.to_owned());
        };
        let relative = path.strip_prefix(server_path).map_err(|_| {
            crate::Error::InvalidPayload(format!(
                "local Bot API file path {:?} is outside server base {:?}",
                path, server_path
            ))
        })?;
        Ok(local_path.join(relative))
    }

    pub fn to_server(&self, path: impl AsRef<Path>) -> Result<PathBuf> {
        let path = path.as_ref();
        let Some((server_path, local_path)) = &self.local_paths else {
            return Ok(path.to_owned());
        };
        let relative = path.strip_prefix(local_path).map_err(|_| {
            crate::Error::InvalidPayload(format!(
                "local file path {:?} is outside local base {:?}",
                path, local_path
            ))
        })?;
        Ok(server_path.join(relative))
    }
}

/// A prepared Bot API request passed through client middleware.
#[derive(Debug, Clone)]
pub struct BotRequest {
    pub method_name: String,
    pub payload: Value,
    pub(crate) files: Vec<InputFileUpload>,
}

impl BotRequest {
    pub(crate) fn new(
        method_name: impl Into<String>,
        payload: Value,
        files: Vec<InputFileUpload>,
    ) -> Self {
        Self {
            method_name: method_name.into(),
            payload,
            files,
        }
    }

    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }
}

/// Remaining request-middleware chain. Middleware must call `run` to execute
/// the next layer and eventually send the HTTP request.
#[derive(Clone)]
pub struct RequestNext {
    pub(crate) middlewares: Arc<Vec<Arc<dyn RequestMiddleware>>>,
    pub(crate) index: usize,
}

impl RequestNext {
    pub(crate) fn new(middlewares: Arc<Vec<Arc<dyn RequestMiddleware>>>) -> Self {
        Self {
            middlewares,
            index: 0,
        }
    }

    pub async fn run(self, bot: Bot, request: BotRequest) -> Result<Value> {
        if let Some(middleware) = self.middlewares.get(self.index).cloned() {
            let next = Self {
                index: self.index + 1,
                ..self
            };
            middleware.handle(bot, request, next).await
        } else {
            bot.send_request(request).await
        }
    }
}

/// Outgoing Bot API middleware, analogous to aiogram client-session middleware.
#[async_trait]
pub trait RequestMiddleware: Send + Sync {
    async fn handle(&self, bot: Bot, request: BotRequest, next: RequestNext) -> Result<Value>;
}

/// Logs outgoing Bot API method names without exposing the bot token.
#[derive(Debug, Clone, Default)]
pub struct RequestLogging {
    ignored_methods: BTreeSet<String>,
}

impl RequestLogging {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ignore(mut self, method_name: impl Into<String>) -> Self {
        self.ignored_methods.insert(method_name.into());
        self
    }
}

#[async_trait]
impl RequestMiddleware for RequestLogging {
    async fn handle(&self, bot: Bot, request: BotRequest, next: RequestNext) -> Result<Value> {
        if !self.ignored_methods.contains(&request.method_name) {
            tracing::info!(
                method = %request.method_name,
                bot_id = bot.id(),
                "making Telegram Bot API request"
            );
        }
        next.run(bot, request).await
    }
}

/// Values automatically applied when a method supports the corresponding
/// optional field and the field was not explicitly set.
#[derive(Debug, Clone, Default)]
pub struct DefaultBotProperties {
    pub parse_mode: Option<String>,
    pub disable_notification: Option<bool>,
    pub protect_content: Option<bool>,
    pub allow_sending_without_reply: Option<bool>,
    pub link_preview: Option<LinkPreviewOptions>,
    pub link_preview_is_disabled: Option<bool>,
    pub link_preview_prefer_small_media: Option<bool>,
    pub link_preview_prefer_large_media: Option<bool>,
    pub link_preview_show_above_text: Option<bool>,
    pub show_caption_above_media: Option<bool>,
}

impl DefaultBotProperties {
    pub fn parse_mode(mut self, value: impl Into<String>) -> Self {
        self.parse_mode = Some(value.into());
        self
    }

    pub fn disable_notification(mut self, value: bool) -> Self {
        self.disable_notification = Some(value);
        self
    }

    pub fn protect_content(mut self, value: bool) -> Self {
        self.protect_content = Some(value);
        self
    }

    pub fn allow_sending_without_reply(mut self, value: bool) -> Self {
        self.allow_sending_without_reply = Some(value);
        self
    }

    pub fn link_preview(mut self, value: LinkPreviewOptions) -> Self {
        self.link_preview = Some(value);
        self
    }

    pub fn link_preview_is_disabled(mut self, value: bool) -> Self {
        self.link_preview_is_disabled = Some(value);
        self
    }

    pub fn link_preview_prefer_small_media(mut self, value: bool) -> Self {
        self.link_preview_prefer_small_media = Some(value);
        self
    }

    pub fn link_preview_prefer_large_media(mut self, value: bool) -> Self {
        self.link_preview_prefer_large_media = Some(value);
        self
    }

    pub fn link_preview_show_above_text(mut self, value: bool) -> Self {
        self.link_preview_show_above_text = Some(value);
        self
    }

    pub fn show_caption_above_media(mut self, value: bool) -> Self {
        self.show_caption_above_media = Some(value);
        self
    }

    pub(crate) fn apply(&self, defaults: &[(&str, &str)], payload: &mut Value) -> Result<()> {
        let Some(object) = payload.as_object_mut() else {
            return Ok(());
        };
        let generated_link_preview = self.generated_link_preview();
        for &(field, property) in defaults {
            match property {
                "parse_mode" => self.insert(object, field, self.parse_mode.as_ref())?,
                "disable_notification" => {
                    self.insert(object, field, self.disable_notification.as_ref())?
                }
                "protect_content" => self.insert(object, field, self.protect_content.as_ref())?,
                "allow_sending_without_reply" => {
                    self.insert(object, field, self.allow_sending_without_reply.as_ref())?
                }
                "link_preview" => self.insert(
                    object,
                    field,
                    self.link_preview
                        .as_ref()
                        .or(generated_link_preview.as_ref()),
                )?,
                "link_preview_is_disabled" => {
                    self.insert(object, field, self.link_preview_is_disabled.as_ref())?
                }
                "link_preview_prefer_small_media" => {
                    self.insert(object, field, self.link_preview_prefer_small_media.as_ref())?
                }
                "link_preview_prefer_large_media" => {
                    self.insert(object, field, self.link_preview_prefer_large_media.as_ref())?
                }
                "link_preview_show_above_text" => {
                    self.insert(object, field, self.link_preview_show_above_text.as_ref())?
                }
                "show_caption_above_media" => {
                    self.insert(object, field, self.show_caption_above_media.as_ref())?
                }
                property => {
                    return Err(crate::Error::InvalidPayload(format!(
                        "unsupported aiogram default property {property:?} for field {field:?}"
                    )));
                }
            }
        }
        // A flattened explicit JSON null is an internal opt-out marker. It
        // prevents a client default from being inserted, but is removed before
        // the payload reaches Telegram.
        object.retain(|_, value| !value.is_null());
        Ok(())
    }

    fn generated_link_preview(&self) -> Option<LinkPreviewOptions> {
        if ![
            self.link_preview_is_disabled,
            self.link_preview_prefer_small_media,
            self.link_preview_prefer_large_media,
            self.link_preview_show_above_text,
        ]
        .into_iter()
        .any(|value| value == Some(true))
        {
            return None;
        }
        let mut options = LinkPreviewOptions::new();
        if let Some(value) = self.link_preview_is_disabled {
            options = options.is_disabled(value);
        }
        if let Some(value) = self.link_preview_prefer_small_media {
            options = options.prefer_small_media(value);
        }
        if let Some(value) = self.link_preview_prefer_large_media {
            options = options.prefer_large_media(value);
        }
        if let Some(value) = self.link_preview_show_above_text {
            options = options.show_above_text(value);
        }
        Some(options)
    }

    fn insert<T: serde::Serialize>(
        &self,
        object: &mut serde_json::Map<String, Value>,
        name: &str,
        value: Option<&T>,
    ) -> Result<()> {
        if !object.contains_key(name)
            && let Some(value) = value
        {
            object.insert(name.to_owned(), serde_json::to_value(value)?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_api_server_builds_production_test_and_local_paths() {
        let production = TelegramApiServer::production();
        assert_eq!(
            production.api_url("token", "getMe"),
            "https://api.telegram.org/bottoken/getMe"
        );
        assert_eq!(
            TelegramApiServer::test().api_url("token", "getMe"),
            "https://api.telegram.org/bottoken/test/getMe"
        );
        let local = TelegramApiServer::from_base("http://localhost:8081/")
            .local_file_paths("/srv/telegram", "/var/lib/telegram");
        assert_eq!(
            local.to_local("/srv/telegram/photos/a.jpg").unwrap(),
            PathBuf::from("/var/lib/telegram/photos/a.jpg")
        );
        assert_eq!(
            local.to_server("/var/lib/telegram/photos/a.jpg").unwrap(),
            PathBuf::from("/srv/telegram/photos/a.jpg")
        );
    }
}
