//! Ergonomic helpers corresponding to commonly used `aiogram.utils` features.

pub mod token {
    use crate::error::{Error, Result};

    pub fn validate(value: &str) -> Result<()> {
        if value.chars().any(char::is_whitespace) {
            return Err(Error::InvalidToken(
                "token must not contain whitespace".to_owned(),
            ));
        }
        let (id, secret) = value
            .split_once(':')
            .ok_or_else(|| Error::InvalidToken("expected '<bot id>:<secret>'".to_owned()))?;
        if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) || secret.is_empty() {
            return Err(Error::InvalidToken("invalid Telegram bot token".to_owned()));
        }
        Ok(())
    }

    pub fn extract_bot_id(value: &str) -> Result<i64> {
        validate(value)?;
        value
            .split_once(':')
            .and_then(|(id, _)| id.parse().ok())
            .ok_or_else(|| Error::InvalidToken("bot id does not fit in i64".to_owned()))
    }
}

pub mod callback_answer {
    use std::fmt;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use crate::dispatcher::{Middleware, Next, UpdateContext};
    use crate::error::{Error, Result};
    use crate::methods::AnswerCallbackQuery;

    #[derive(Debug, Clone, Default)]
    struct CallbackAnswerState {
        answered: bool,
        disabled: bool,
        text: Option<String>,
        show_alert: Option<bool>,
        url: Option<String>,
        cache_time: Option<i64>,
    }

    /// Per-handler overrides for [`CallbackAnswerMiddleware`]. Store this in a
    /// `callback_answer` [`crate::HandlerFlags`] entry.
    #[derive(Debug, Clone, Default)]
    pub struct CallbackAnswerConfig {
        pre: Option<bool>,
        disabled: Option<bool>,
        text: Option<Option<String>>,
        show_alert: Option<Option<bool>>,
        url: Option<Option<String>>,
        cache_time: Option<Option<i64>>,
    }

    impl CallbackAnswerConfig {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn pre(mut self, value: bool) -> Self {
            self.pre = Some(value);
            self
        }

        pub fn disabled(mut self, value: bool) -> Self {
            self.disabled = Some(value);
            self
        }

        pub fn text(mut self, value: impl Into<String>) -> Self {
            self.text = Some(Some(value.into()));
            self
        }

        pub fn clear_text(mut self) -> Self {
            self.text = Some(None);
            self
        }

        pub fn show_alert(mut self, value: bool) -> Self {
            self.show_alert = Some(Some(value));
            self
        }

        pub fn clear_show_alert(mut self) -> Self {
            self.show_alert = Some(None);
            self
        }

        pub fn url(mut self, value: impl Into<String>) -> Self {
            self.url = Some(Some(value.into()));
            self
        }

        pub fn clear_url(mut self) -> Self {
            self.url = Some(None);
            self
        }

        pub fn cache_time(mut self, value: i64) -> Self {
            self.cache_time = Some(Some(value));
            self
        }

        pub fn clear_cache_time(mut self) -> Self {
            self.cache_time = Some(None);
            self
        }
    }

    /// Per-handler callback answer configuration injected by
    /// [`CallbackAnswerMiddleware`].
    #[derive(Debug, Clone, Default)]
    pub struct CallbackAnswer(Arc<Mutex<CallbackAnswerState>>);

    impl CallbackAnswer {
        fn configured(state: CallbackAnswerState) -> Self {
            Self(Arc::new(Mutex::new(state)))
        }

        pub fn answered(&self) -> bool {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .answered
        }

        pub fn disabled(&self) -> bool {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .disabled
        }

        pub fn text_value(&self) -> Option<String> {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .text
                .clone()
        }

        pub fn show_alert_value(&self) -> Option<bool> {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .show_alert
        }

        pub fn url_value(&self) -> Option<String> {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .url
                .clone()
        }

        pub fn cache_time_value(&self) -> Option<i64> {
            self.0
                .lock()
                .expect("callback answer lock poisoned")
                .cache_time
        }

        pub fn disable(&self) -> Result<()> {
            self.mutate_before_answer(|state| state.disabled = true)
        }

        pub fn text(&self, value: impl Into<String>) -> Result<()> {
            self.mutate_before_answer(|state| state.text = Some(value.into()))
        }

        pub fn clear_text(&self) -> Result<()> {
            self.mutate_before_answer(|state| state.text = None)
        }

        pub fn show_alert(&self, value: bool) -> Result<()> {
            self.mutate_before_answer(|state| state.show_alert = Some(value))
        }

        pub fn clear_show_alert(&self) -> Result<()> {
            self.mutate_before_answer(|state| state.show_alert = None)
        }

        pub fn url(&self, value: impl Into<String>) -> Result<()> {
            self.mutate_before_answer(|state| state.url = Some(value.into()))
        }

        pub fn clear_url(&self) -> Result<()> {
            self.mutate_before_answer(|state| state.url = None)
        }

        pub fn cache_time(&self, value: i64) -> Result<()> {
            self.mutate_before_answer(|state| state.cache_time = Some(value))
        }

        pub fn clear_cache_time(&self) -> Result<()> {
            self.mutate_before_answer(|state| state.cache_time = None)
        }

        fn mutate_before_answer(
            &self,
            mutate: impl FnOnce(&mut CallbackAnswerState),
        ) -> Result<()> {
            let mut state = self
                .0
                .lock()
                .map_err(|_| Error::Handler("callback answer lock poisoned".to_owned()))?;
            if state.answered {
                return Err(Error::Handler(
                    "callback answer cannot be changed after it was sent".to_owned(),
                ));
            }
            mutate(&mut state);
            Ok(())
        }

        fn snapshot(&self) -> Result<CallbackAnswerState> {
            self.0
                .lock()
                .map(|state| state.clone())
                .map_err(|_| Error::Handler("callback answer lock poisoned".to_owned()))
        }

        fn mark_answered(&self) -> Result<()> {
            self.0
                .lock()
                .map(|mut state| state.answered = true)
                .map_err(|_| Error::Handler("callback answer lock poisoned".to_owned()))
        }
    }

    impl fmt::Display for CallbackAnswer {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            let state = self.0.lock().map_err(|_| fmt::Error)?;
            write!(
                formatter,
                "CallbackAnswer(answered={}, disabled={}",
                state.answered, state.disabled
            )?;
            if let Some(text) = &state.text {
                write!(formatter, ", text={text:?}")?;
            }
            if let Some(show_alert) = state.show_alert {
                write!(formatter, ", show_alert={show_alert}")?;
            }
            if let Some(url) = &state.url {
                write!(formatter, ", url={url:?}")?;
            }
            if let Some(cache_time) = state.cache_time {
                write!(formatter, ", cache_time={cache_time}")?;
            }
            formatter.write_str(")")
        }
    }

    /// Automatically answers callback queries before or after their handler.
    #[derive(Debug, Clone, Default)]
    pub struct CallbackAnswerMiddleware {
        pre: bool,
        text: Option<String>,
        show_alert: Option<bool>,
        url: Option<String>,
        cache_time: Option<i64>,
    }

    impl CallbackAnswerMiddleware {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn pre(mut self, value: bool) -> Self {
            self.pre = value;
            self
        }

        pub fn text(mut self, value: impl Into<String>) -> Self {
            self.text = Some(value.into());
            self
        }

        pub fn show_alert(mut self, value: bool) -> Self {
            self.show_alert = Some(value);
            self
        }

        pub fn url(mut self, value: impl Into<String>) -> Self {
            self.url = Some(value.into());
            self
        }

        pub fn cache_time(mut self, value: i64) -> Self {
            self.cache_time = Some(value);
            self
        }

        async fn answer(context: &UpdateContext, answer: &CallbackAnswer) -> Result<()> {
            let query = context.callback_query().ok_or_else(|| {
                Error::Handler("callback answer requires a callback query".to_owned())
            })?;
            let state = answer.snapshot()?;
            if state.disabled || state.answered {
                return Ok(());
            }
            let mut method = AnswerCallbackQuery::new(query.id.clone());
            if let Some(text) = state.text {
                method = method.text(text);
            }
            if let Some(show_alert) = state.show_alert {
                method = method.show_alert(show_alert);
            }
            if let Some(url) = state.url {
                method = method.url(url);
            }
            if let Some(cache_time) = state.cache_time {
                method = method.cache_time(cache_time);
            }
            context.bot.execute(&method).await?;
            answer.mark_answered()
        }
    }

    #[async_trait]
    impl Middleware for CallbackAnswerMiddleware {
        async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
            if context.callback_query().is_none() {
                return next.run(context).await;
            }
            let overrides = context.handler_flag::<CallbackAnswerConfig>("callback_answer");
            let pre = overrides
                .as_ref()
                .and_then(|config| config.pre)
                .unwrap_or(self.pre);
            let answer = CallbackAnswer::configured(CallbackAnswerState {
                answered: false,
                disabled: overrides
                    .as_ref()
                    .and_then(|config| config.disabled)
                    .unwrap_or(false),
                text: overrides
                    .as_ref()
                    .and_then(|config| config.text.clone())
                    .unwrap_or_else(|| self.text.clone()),
                show_alert: overrides
                    .as_ref()
                    .and_then(|config| config.show_alert)
                    .unwrap_or(self.show_alert),
                url: overrides
                    .as_ref()
                    .and_then(|config| config.url.clone())
                    .unwrap_or_else(|| self.url.clone()),
                cache_time: overrides
                    .as_ref()
                    .and_then(|config| config.cache_time)
                    .unwrap_or(self.cache_time),
            });
            if pre {
                Self::answer(&context, &answer).await?;
            }
            let handler_result = next
                .run(context.clone().with_dependency(answer.clone()))
                .await;
            let answer_result = Self::answer(&context, &answer).await;
            match (handler_result, answer_result) {
                (Err(error), _) => Err(error),
                (Ok(()), result) => result,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        use super::*;
        use crate::{Bot, Dispatcher, HandlerFlags, Router, filters};

        async fn mock_server() -> (String, tokio::task::JoinHandle<String>) {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                let mut expected = None;
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    request.extend_from_slice(&buffer[..read]);
                    if expected.is_none()
                        && let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or_default();
                        expected = Some(end + 4 + length);
                    }
                    if read == 0 || expected.is_some_and(|length| request.len() >= length) {
                        break;
                    }
                }
                let body = r#"{"ok":true,"result":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                String::from_utf8(request).unwrap()
            });
            (format!("http://{address}"), task)
        }

        #[tokio::test]
        async fn middleware_answers_after_handler_with_mutated_configuration() {
            let (api_base, request) = mock_server().await;
            let bot =
                Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
            let mut router = Router::new();
            router.middleware(CallbackAnswerMiddleware::new());
            router.callback_query(filters::any(), |context| async move {
                context
                    .dependency::<CallbackAnswer>()
                    .unwrap()
                    .text("handled")?;
                Ok(())
            });
            let mut dispatcher = Dispatcher::new();
            dispatcher.include_router(router);
            let update = serde_json::from_value(serde_json::json!({
                "update_id": 1,
                "callback_query": {
                    "id": "callback-1",
                    "from": {"id": 1, "is_bot": false, "first_name": "Ada"},
                    "chat_instance": "instance",
                    "data": "action"
                }
            }))
            .unwrap();

            assert!(dispatcher.feed_update(bot, update).await.unwrap());
            let request = request.await.unwrap();
            assert!(request.contains("/answerCallbackQuery "));
            assert!(request.contains(r#""callback_query_id":"callback-1""#));
            assert!(request.contains(r#""text":"handled""#));
        }

        #[test]
        fn callback_answer_values_are_readable_and_lock_after_answering() {
            let answer = CallbackAnswer::configured(CallbackAnswerState::default());
            answer.text("ready").unwrap();
            answer.show_alert(true).unwrap();
            answer.url("https://example.com").unwrap();
            answer.cache_time(15).unwrap();

            assert_eq!(answer.text_value().as_deref(), Some("ready"));
            assert_eq!(answer.show_alert_value(), Some(true));
            assert_eq!(answer.url_value().as_deref(), Some("https://example.com"));
            assert_eq!(answer.cache_time_value(), Some(15));
            assert!(!answer.answered());
            assert!(!answer.disabled());
            assert_eq!(
                answer.to_string(),
                "CallbackAnswer(answered=false, disabled=false, text=\"ready\", show_alert=true, url=\"https://example.com\", cache_time=15)"
            );

            answer.clear_text().unwrap();
            answer.clear_show_alert().unwrap();
            answer.clear_url().unwrap();
            answer.clear_cache_time().unwrap();
            assert_eq!(answer.text_value(), None);
            assert_eq!(answer.show_alert_value(), None);
            assert_eq!(answer.url_value(), None);
            assert_eq!(answer.cache_time_value(), None);
            answer.text("ready").unwrap();

            answer.mark_answered().unwrap();
            assert!(answer.answered());
            assert!(answer.text("too late").is_err());
            assert_eq!(answer.text_value().as_deref(), Some("ready"));
        }

        #[tokio::test]
        async fn handler_flag_overrides_callback_answer_defaults() {
            let (api_base, request) = mock_server().await;
            let bot =
                Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
            let mut router = Router::new();
            router.middleware(CallbackAnswerMiddleware::new().text("default"));
            router.event_with_flags(
                "callback_query",
                filters::any(),
                HandlerFlags::new().with(
                    "callback_answer",
                    CallbackAnswerConfig::new().pre(true).text("from flag"),
                ),
                |_| async { Ok(()) },
            );
            let mut dispatcher = Dispatcher::new();
            dispatcher.include_router(router);
            let update = serde_json::from_value(serde_json::json!({
                "update_id": 1,
                "callback_query": {
                    "id": "callback-flag",
                    "from": {"id": 1, "is_bot": false, "first_name": "Ada"},
                    "chat_instance": "instance",
                    "data": "action"
                }
            }))
            .unwrap();

            assert!(dispatcher.feed_update(bot, update).await.unwrap());
            let request = request.await.unwrap();
            assert!(request.contains(r#""text":"from flag""#));
            assert!(!request.contains(r#""text":"default""#));
        }

        #[tokio::test]
        async fn handler_flag_can_clear_callback_answer_defaults() {
            let (api_base, request) = mock_server().await;
            let bot =
                Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
            let mut router = Router::new();
            router.middleware(
                CallbackAnswerMiddleware::new()
                    .text("default")
                    .show_alert(true)
                    .url("https://example.com")
                    .cache_time(30),
            );
            router.event_with_flags(
                "callback_query",
                filters::any(),
                HandlerFlags::new().with(
                    "callback_answer",
                    CallbackAnswerConfig::new()
                        .pre(true)
                        .clear_text()
                        .clear_show_alert()
                        .clear_url()
                        .clear_cache_time(),
                ),
                |context| async move {
                    let answer = context.dependency::<CallbackAnswer>().unwrap();
                    assert_eq!(answer.text_value(), None);
                    assert_eq!(answer.show_alert_value(), None);
                    assert_eq!(answer.url_value(), None);
                    assert_eq!(answer.cache_time_value(), None);
                    Ok(())
                },
            );
            let mut dispatcher = Dispatcher::new();
            dispatcher.include_router(router);
            let update = serde_json::from_value(serde_json::json!({
                "update_id": 1,
                "callback_query": {
                    "id": "callback-clear",
                    "from": {"id": 1, "is_bot": false, "first_name": "Ada"},
                    "chat_instance": "instance",
                    "data": "action"
                }
            }))
            .unwrap();

            assert!(dispatcher.feed_update(bot, update).await.unwrap());
            let request = request.await.unwrap();
            assert!(!request.contains(r#""text":"default""#));
            assert!(!request.contains(r#""show_alert":true"#));
            assert!(!request.contains(r#""url":"https://example.com""#));
            assert!(!request.contains(r#""cache_time":30"#));
        }
    }
}

pub mod web_app {
    use std::collections::BTreeMap;

    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    use hmac::{Hmac, Mac};
    use serde::{Deserialize, Serialize};
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use crate::error::{Error, Result};

    type HmacSha256 = Hmac<Sha256>;

    pub const PRODUCTION_PUBLIC_KEY: [u8; 32] = [
        0xe7, 0xbf, 0x03, 0xa2, 0xfa, 0x46, 0x02, 0xaf, 0x45, 0x80, 0x70, 0x3d, 0x88, 0xdd, 0xa5,
        0xbb, 0x59, 0xf3, 0x2e, 0xd8, 0xb0, 0x2a, 0x56, 0xc1, 0x87, 0xfe, 0x7d, 0x34, 0xca, 0xed,
        0x24, 0x2d,
    ];
    pub const TEST_PUBLIC_KEY: [u8; 32] = [
        0x40, 0x05, 0x50, 0x58, 0xa4, 0xee, 0x38, 0x15, 0x6a, 0x06, 0x56, 0x2e, 0x52, 0xee, 0xce,
        0x92, 0xa7, 0x71, 0xbc, 0xd8, 0x34, 0x6a, 0x8c, 0x46, 0x15, 0xcb, 0x73, 0x76, 0xed, 0xdf,
        0x72, 0xec,
    ];

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct WebAppUser {
        pub id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub is_bot: Option<bool>,
        pub first_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub last_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub language_code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub is_premium: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub added_to_attachment_menu: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub allows_write_to_pm: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub photo_url: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct WebAppChat {
        pub id: i64,
        #[serde(rename = "type")]
        pub kind: String,
        pub title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub username: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub photo_url: Option<String>,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    pub struct WebAppInitData {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub query_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub user: Option<WebAppUser>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub receiver: Option<WebAppUser>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chat: Option<WebAppChat>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chat_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chat_instance: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub start_param: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub can_send_after: Option<i64>,
        pub auth_date: i64,
        pub hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub signature: Option<String>,
        #[serde(flatten, default)]
        pub extra: BTreeMap<String, Value>,
    }

    /// Validates first-party WebApp init data with the bot-token HMAC scheme.
    pub fn check_signature(token: &str, init_data: &str) -> bool {
        let Ok(mut fields) = parse_query(init_data, true) else {
            return false;
        };
        let Some(expected) = fields.remove("hash") else {
            return false;
        };
        let Some(expected) = decode_hex(&expected) else {
            return false;
        };
        let check_string = fields
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut key_mac =
            HmacSha256::new_from_slice(b"WebAppData").expect("HMAC accepts keys of any size");
        key_mac.update(token.as_bytes());
        let secret = key_mac.finalize().into_bytes();
        let mut signature =
            HmacSha256::new_from_slice(&secret).expect("HMAC accepts keys of any size");
        signature.update(check_string.as_bytes());
        signature.verify_slice(&expected).is_ok()
    }

    /// Parses WebApp data without validating its origin.
    pub fn parse_init_data(init_data: &str) -> Result<WebAppInitData> {
        let fields = parse_query(init_data, false)?;
        let mut object = serde_json::Map::new();
        for (key, value) in fields {
            let value = if matches!(key.as_str(), "auth_date" | "can_send_after") {
                Value::Number(
                    value
                        .parse::<i64>()
                        .map_err(|_| {
                            Error::Utility(format!("WebApp field {key} must be an integer"))
                        })?
                        .into(),
                )
            } else if (value.starts_with('{') && value.ends_with('}'))
                || (value.starts_with('[') && value.ends_with(']'))
            {
                serde_json::from_str(&value)?
            } else {
                Value::String(value)
            };
            object.insert(key, value);
        }
        Ok(serde_json::from_value(Value::Object(object))?)
    }

    pub fn safe_parse_init_data(token: &str, init_data: &str) -> Result<WebAppInitData> {
        if !check_signature(token, init_data) {
            return Err(Error::Utility(
                "invalid WebApp init data signature".to_owned(),
            ));
        }
        parse_init_data(init_data)
    }

    /// Validates WebApp init data for third-party use with Telegram's Ed25519
    /// signature and the production public key.
    pub fn check_signature_with_bot_id(bot_id: i64, init_data: &str) -> bool {
        check_signature_with_public_key(bot_id, init_data, &PRODUCTION_PUBLIC_KEY)
    }

    pub fn check_signature_with_public_key(
        bot_id: i64,
        init_data: &str,
        public_key: &[u8; 32],
    ) -> bool {
        let Ok(mut fields) = parse_query(init_data, true) else {
            return false;
        };
        let Some(signature) = fields.remove("signature") else {
            return false;
        };
        fields.remove("hash");
        let message = format!(
            "{bot_id}:WebAppData\n{}",
            fields
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let padded = format!("{signature}{}", "=".repeat((4 - signature.len() % 4) % 4));
        let Ok(signature) = base64::engine::general_purpose::URL_SAFE.decode(padded) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(&signature) else {
            return false;
        };
        let Ok(public_key) = VerifyingKey::from_bytes(public_key) else {
            return false;
        };
        public_key.verify(message.as_bytes(), &signature).is_ok()
    }

    pub fn safe_parse_init_data_with_bot_id(
        bot_id: i64,
        init_data: &str,
    ) -> Result<WebAppInitData> {
        safe_parse_init_data_with_public_key(bot_id, init_data, &PRODUCTION_PUBLIC_KEY)
    }

    pub fn safe_parse_init_data_with_public_key(
        bot_id: i64,
        init_data: &str,
        public_key: &[u8; 32],
    ) -> Result<WebAppInitData> {
        if !check_signature_with_public_key(bot_id, init_data, public_key) {
            return Err(Error::Utility(
                "invalid third-party WebApp signature".to_owned(),
            ));
        }
        parse_init_data(init_data)
    }

    /// Login Widget HMAC verification.
    pub fn check_login_widget_integrity(token: &str, fields: &BTreeMap<String, String>) -> bool {
        let Some(expected) = fields.get("hash").and_then(|value| decode_hex(value)) else {
            return false;
        };
        let check_string = fields
            .iter()
            .filter(|(key, _)| key.as_str() != "hash")
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let secret = Sha256::digest(token.as_bytes());
        let mut signature =
            HmacSha256::new_from_slice(&secret).expect("HMAC accepts keys of any size");
        signature.update(check_string.as_bytes());
        signature.verify_slice(&expected).is_ok()
    }

    fn parse_query(init_data: &str, strict: bool) -> Result<BTreeMap<String, String>> {
        if strict
            && init_data
                .split('&')
                .any(|part| !part.contains('=') || !valid_percent_encoding(part.as_bytes()))
        {
            return Err(Error::Utility("invalid WebApp query string".to_owned()));
        }
        Ok(url::form_urlencoded::parse(init_data.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect())
    }

    fn valid_percent_encoding(value: &[u8]) -> bool {
        let mut index = 0;
        while index < value.len() {
            if value[index] == b'%' {
                if index + 2 >= value.len()
                    || !value[index + 1].is_ascii_hexdigit()
                    || !value[index + 2].is_ascii_hexdigit()
                {
                    return false;
                }
                index += 3;
            } else {
                index += 1;
            }
        }
        true
    }

    fn decode_hex(value: &str) -> Option<Vec<u8>> {
        if !value.len().is_multiple_of(2) {
            return None;
        }
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let high = (pair[0] as char).to_digit(16)?;
                let low = (pair[1] as char).to_digit(16)?;
                Some(((high << 4) | low) as u8)
            })
            .collect()
    }
}

pub mod media_group {
    use crate::error::{Error, Result};
    use crate::types::{
        InputFile, InputMediaAudio, InputMediaDocument, InputMediaPhoto, InputMediaVideo,
        MediaUnion, MessageEntity,
    };

    pub const MAX_MEDIA_GROUP_SIZE: usize = 10;

    /// Fluent builder for Telegram albums with aiogram-compatible caption handling.
    #[derive(Debug, Clone, Default)]
    pub struct MediaGroupBuilder {
        media: Vec<MediaUnion>,
        caption: Option<String>,
        caption_entities: Option<Vec<MessageEntity>>,
    }

    impl MediaGroupBuilder {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn caption(mut self, value: impl Into<String>) -> Self {
            self.caption = Some(value.into());
            self
        }

        pub fn caption_entities(mut self, value: Vec<MessageEntity>) -> Self {
            self.caption_entities = Some(value);
            self
        }

        #[allow(clippy::should_implement_trait)]
        pub fn add(mut self, value: impl Into<MediaUnion>) -> Result<Self> {
            if self.media.len() >= MAX_MEDIA_GROUP_SIZE {
                return Err(Error::Utility(format!(
                    "media group cannot contain more than {MAX_MEDIA_GROUP_SIZE} items"
                )));
            }
            self.media.push(value.into());
            Ok(self)
        }

        pub fn add_photo(self, media: impl Into<InputFile>) -> Result<Self> {
            self.add(InputMediaPhoto::new(media))
        }

        pub fn add_video(self, media: impl Into<InputFile>) -> Result<Self> {
            self.add(InputMediaVideo::new(media))
        }

        pub fn add_audio(self, media: impl Into<InputFile>) -> Result<Self> {
            self.add(InputMediaAudio::new(media))
        }

        pub fn add_document(self, media: impl Into<InputFile>) -> Result<Self> {
            self.add(InputMediaDocument::new(media))
        }

        pub fn build(mut self) -> Vec<MediaUnion> {
            if let Some(first) = self.media.first_mut()
                && let Some(caption) = self.caption
            {
                apply_caption(first, caption, self.caption_entities);
            }
            self.media
        }

        pub fn len(&self) -> usize {
            self.media.len()
        }

        pub fn is_empty(&self) -> bool {
            self.media.is_empty()
        }
    }

    fn apply_caption(
        media: &mut MediaUnion,
        caption: String,
        entities: Option<Vec<MessageEntity>>,
    ) {
        macro_rules! update {
            ($value:expr) => {{
                $value.caption = Some(caption);
                if let Some(entities) = entities {
                    $value.caption_entities = Some(entities);
                    $value.parse_mode = None;
                }
            }};
        }
        match media {
            MediaUnion::InputMediaAudio(value) => update!(value),
            MediaUnion::InputMediaDocument(value) => update!(value),
            MediaUnion::InputMediaPhoto(value) => update!(value),
            MediaUnion::InputMediaVideo(value) => update!(value),
            MediaUnion::InputMediaLivePhoto(value) => update!(value),
        }
    }
}

pub mod chat_action {
    use std::time::Duration;

    use async_trait::async_trait;
    use tokio::sync::watch;
    use tokio::task::JoinHandle;

    use crate::bot::Bot;
    use crate::dispatcher::{Middleware, Next, UpdateContext};
    use crate::enums::ChatAction;
    use crate::error::{Error, Result};
    use crate::methods::SendChatAction;
    use crate::types::ChatId;

    pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);

    /// Per-handler settings for [`ChatActionMiddleware`]. Store this under the
    /// `chat_action` handler flag.
    #[derive(Debug, Clone, Default)]
    pub struct ChatActionConfig {
        action: Option<String>,
        interval: Option<Duration>,
        initial_sleep: Option<Duration>,
    }

    impl ChatActionConfig {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn action(mut self, value: impl Into<String>) -> Self {
            self.action = Some(value.into());
            self
        }

        pub fn interval(mut self, value: Duration) -> Self {
            self.interval = Some(value);
            self
        }

        pub fn initial_sleep(mut self, value: Duration) -> Self {
            self.initial_sleep = Some(value);
            self
        }
    }

    /// Periodically sends a chat action while a long-running operation is active.
    pub struct ChatActionSender {
        bot: Bot,
        chat_id: ChatId,
        message_thread_id: Option<i64>,
        action: String,
        interval: Duration,
        initial_sleep: Duration,
        stop: Option<watch::Sender<bool>>,
        task: Option<JoinHandle<()>>,
    }

    impl ChatActionSender {
        pub fn new(bot: Bot, chat_id: impl Into<ChatId>, action: impl Into<String>) -> Self {
            Self {
                bot,
                chat_id: chat_id.into(),
                message_thread_id: None,
                action: action.into(),
                interval: DEFAULT_INTERVAL,
                initial_sleep: Duration::ZERO,
                stop: None,
                task: None,
            }
        }

        pub fn typing(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::Typing)
        }

        pub fn upload_photo(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::UploadPhoto)
        }

        pub fn record_video(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::RecordVideo)
        }

        pub fn upload_video(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::UploadVideo)
        }

        pub fn record_voice(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::RecordVoice)
        }

        pub fn upload_voice(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::UploadVoice)
        }

        pub fn upload_document(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::UploadDocument)
        }

        pub fn choose_sticker(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::ChooseSticker)
        }

        pub fn find_location(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::FindLocation)
        }

        pub fn record_video_note(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::RecordVideoNote)
        }

        pub fn upload_video_note(bot: Bot, chat_id: impl Into<ChatId>) -> Self {
            Self::new(bot, chat_id, ChatAction::UploadVideoNote)
        }

        pub fn bot(&self) -> &Bot {
            &self.bot
        }

        pub fn chat_id(&self) -> &ChatId {
            &self.chat_id
        }

        pub fn message_thread(&self) -> Option<i64> {
            self.message_thread_id
        }

        pub fn action(&self) -> &str {
            &self.action
        }

        pub fn repeat_interval(&self) -> Duration {
            self.interval
        }

        pub fn initial_delay(&self) -> Duration {
            self.initial_sleep
        }

        pub fn message_thread_id(mut self, value: i64) -> Self {
            self.message_thread_id = Some(value);
            self
        }

        pub fn interval(mut self, value: Duration) -> Self {
            self.interval = value;
            self
        }

        pub fn initial_sleep(mut self, value: Duration) -> Self {
            self.initial_sleep = value;
            self
        }

        pub fn is_running(&self) -> bool {
            self.task.as_ref().is_some_and(|task| !task.is_finished())
        }

        pub fn start(&mut self) -> Result<()> {
            if self.is_running() {
                return Err(Error::Utility(
                    "chat action sender is already running".to_owned(),
                ));
            }
            let (stop, mut stopped) = watch::channel(false);
            let bot = self.bot.clone();
            let chat_id = self.chat_id.clone();
            let action = self.action.clone();
            let thread_id = self.message_thread_id;
            let interval = self.interval;
            let initial_sleep = self.initial_sleep;
            self.stop = Some(stop);
            self.task = Some(tokio::spawn(async move {
                if wait_or_stop(initial_sleep, &mut stopped).await {
                    return;
                }
                loop {
                    let started = tokio::time::Instant::now();
                    let mut method = SendChatAction::new(chat_id.clone(), action.clone());
                    if let Some(thread_id) = thread_id {
                        method = method.message_thread_id(thread_id);
                    }
                    if let Err(error) = bot.execute(&method).await {
                        tracing::warn!(%error, "failed to send periodic chat action");
                    }
                    let sleep = interval.saturating_sub(started.elapsed());
                    if wait_or_stop(sleep, &mut stopped).await {
                        return;
                    }
                }
            }));
            Ok(())
        }

        pub async fn stop(&mut self) -> Result<()> {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(true);
            }
            if let Some(task) = self.task.take() {
                task.await
                    .map_err(|error| Error::Utility(format!("chat action task failed: {error}")))?;
            }
            Ok(())
        }
    }

    impl Drop for ChatActionSender {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(true);
            }
            if let Some(task) = self.task.take() {
                task.abort();
            }
        }
    }

    /// Wraps every message handler in a periodic chat-action sender. Defaults
    /// to `typing` and accepts per-handler overrides via [`ChatActionConfig`].
    #[derive(Debug, Clone, Default)]
    pub struct ChatActionMiddleware {
        defaults: ChatActionConfig,
    }

    impl ChatActionMiddleware {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn action(mut self, value: impl Into<String>) -> Self {
            self.defaults.action = Some(value.into());
            self
        }

        pub fn interval(mut self, value: Duration) -> Self {
            self.defaults.interval = Some(value);
            self
        }

        pub fn initial_sleep(mut self, value: Duration) -> Self {
            self.defaults.initial_sleep = Some(value);
            self
        }
    }

    #[async_trait]
    impl Middleware for ChatActionMiddleware {
        async fn handle(&self, context: UpdateContext, next: Next) -> Result<()> {
            let Some(message) = context.message() else {
                return next.run(context).await;
            };
            let overrides = context.handler_flag::<ChatActionConfig>("chat_action");
            let action = overrides
                .as_ref()
                .and_then(|config| config.action.clone())
                .or_else(|| self.defaults.action.clone())
                .unwrap_or_else(|| ChatAction::Typing.to_string());
            let interval = overrides
                .as_ref()
                .and_then(|config| config.interval)
                .or(self.defaults.interval)
                .unwrap_or(DEFAULT_INTERVAL);
            let initial_sleep = overrides
                .as_ref()
                .and_then(|config| config.initial_sleep)
                .or(self.defaults.initial_sleep)
                .unwrap_or(Duration::ZERO);
            let mut sender = ChatActionSender::new(context.bot.clone(), message.chat.id, action)
                .interval(interval)
                .initial_sleep(initial_sleep);
            if message.is_topic_message == Some(true)
                && let Some(thread_id) = message.message_thread_id
            {
                sender = sender.message_thread_id(thread_id);
            }
            sender.start()?;
            let handler_result = next.run(context).await;
            let stop_result = sender.stop().await;
            match (handler_result, stop_result) {
                (Err(error), _) => Err(error),
                (Ok(()), result) => result,
            }
        }
    }

    async fn wait_or_stop(duration: Duration, stopped: &mut watch::Receiver<bool>) -> bool {
        if *stopped.borrow() {
            return true;
        }
        tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            changed = stopped.changed() => changed.is_err() || *stopped.borrow(),
        }
    }
}

pub mod backoff {
    use std::fmt;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::error::{Error, Result};

    /// Retry timing compatible with aiogram's polling defaults.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct BackoffConfig {
        pub min_delay: Duration,
        pub max_delay: Duration,
        pub factor: f64,
        pub jitter: Duration,
    }

    impl BackoffConfig {
        pub fn new(
            min_delay: Duration,
            max_delay: Duration,
            factor: f64,
            jitter: Duration,
        ) -> Result<Self> {
            if max_delay <= min_delay {
                return Err(Error::Utility(
                    "backoff max_delay must be greater than min_delay".to_owned(),
                ));
            }
            if factor <= 1.0 || !factor.is_finite() {
                return Err(Error::Utility(
                    "backoff factor must be finite and greater than 1".to_owned(),
                ));
            }
            Ok(Self {
                min_delay,
                max_delay,
                factor,
                jitter,
            })
        }
    }

    impl Default for BackoffConfig {
        fn default() -> Self {
            Self {
                min_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(5),
                factor: 1.3,
                jitter: Duration::from_millis(100),
            }
        }
    }

    /// Stateful exponential retry schedule. Jitter uses a private lightweight
    /// generator so the framework does not impose an RNG dependency on users.
    #[derive(Debug, Clone)]
    pub struct Backoff {
        config: BackoffConfig,
        next_delay: Duration,
        current_delay: Duration,
        counter: u64,
        rng_state: u64,
    }

    impl Backoff {
        pub fn new(config: BackoffConfig) -> Self {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            Self {
                next_delay: config.min_delay,
                current_delay: Duration::ZERO,
                counter: 0,
                config,
                rng_state: seed | 1,
            }
        }

        pub fn config(&self) -> BackoffConfig {
            self.config
        }

        pub fn min_delay(&self) -> Duration {
            self.config.min_delay
        }

        pub fn max_delay(&self) -> Duration {
            self.config.max_delay
        }

        pub fn factor(&self) -> f64 {
            self.config.factor
        }

        pub fn jitter(&self) -> Duration {
            self.config.jitter
        }

        pub fn next_delay(&self) -> Duration {
            self.next_delay
        }

        pub fn current_delay(&self) -> Duration {
            self.current_delay
        }

        pub fn counter(&self) -> u64 {
            self.counter
        }

        pub fn reset(&mut self) {
            self.next_delay = self.config.min_delay;
            self.current_delay = Duration::ZERO;
            self.counter = 0;
        }

        pub async fn sleep(&mut self) {
            tokio::time::sleep(self.advance()).await;
        }

        pub fn advance(&mut self) -> Duration {
            self.current_delay = self.next_delay;
            let maximum = self.config.max_delay.as_secs_f64();
            let base = (self.next_delay.as_secs_f64() * self.config.factor).min(maximum);
            let jitter = self.config.jitter.as_secs_f64();
            let randomized = self.normalvariate(base, jitter).max(0.0);
            self.next_delay = Duration::from_secs_f64(randomized);
            self.counter += 1;
            self.current_delay
        }

        fn unit(&mut self) -> f64 {
            let mut value = self.rng_state;
            value ^= value << 13;
            value ^= value >> 7;
            value ^= value << 17;
            self.rng_state = value;
            (value as f64 + 1.0) / (u64::MAX as f64 + 2.0)
        }

        fn normalvariate(&mut self, mean: f64, standard_deviation: f64) -> f64 {
            if standard_deviation == 0.0 {
                return mean;
            }
            let radius = (-2.0 * self.unit().ln()).sqrt();
            let angle = std::f64::consts::TAU * self.unit();
            mean + standard_deviation * radius * angle.cos()
        }
    }

    impl Iterator for Backoff {
        type Item = Duration;

        fn next(&mut self) -> Option<Self::Item> {
            Some(self.advance())
        }
    }

    impl fmt::Display for Backoff {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "Backoff(tryings={}, current_delay={}, next_delay={})",
                self.counter,
                self.current_delay.as_secs_f64(),
                self.next_delay.as_secs_f64()
            )
        }
    }
}

pub mod callback_data {
    use std::fmt::Display;
    use std::str::FromStr;

    use crate::error::{Error, Result};

    pub const MAX_CALLBACK_LENGTH: usize = 64;

    /// A value that can be represented in aiogram's callback-data protocol.
    ///
    /// `bool` is encoded as `1`/`0` and `Option<T>` uses an empty string for
    /// `None`, matching the Python implementation. Applications can implement
    /// this trait for their own enums and identifier newtypes.
    pub trait CallbackValue: Sized {
        fn encode_callback(&self) -> String;

        fn decode_callback(value: &str) -> Result<Self>;
    }

    /// A callback protocol that can be packed directly into an inline button.
    pub trait PackCallbackData {
        fn pack_callback_data(&self) -> Result<String>;
    }

    impl CallbackValue for String {
        fn encode_callback(&self) -> String {
            self.clone()
        }

        fn decode_callback(value: &str) -> Result<Self> {
            Ok(value.to_owned())
        }
    }

    impl CallbackValue for bool {
        fn encode_callback(&self) -> String {
            if *self { "1" } else { "0" }.to_owned()
        }

        fn decode_callback(value: &str) -> Result<Self> {
            match value {
                "1" => Ok(true),
                "0" => Ok(false),
                _ => Err(Error::Utility(format!(
                    "invalid callback boolean {value:?}; expected `1` or `0`"
                ))),
            }
        }
    }

    macro_rules! impl_callback_value_from_str {
        ($($type:ty),+ $(,)?) => {
            $(
                impl CallbackValue for $type {
                    fn encode_callback(&self) -> String {
                        self.to_string()
                    }

                    fn decode_callback(value: &str) -> Result<Self> {
                        <$type>::from_str(value).map_err(|error| {
                            Error::Utility(format!(
                                "invalid callback {} value {value:?}: {error}",
                                stringify!($type),
                            ))
                        })
                    }
                }
            )+
        };
    }

    impl_callback_value_from_str!(
        i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64,
    );

    impl<T: CallbackValue> CallbackValue for Option<T> {
        fn encode_callback(&self) -> String {
            self.as_ref()
                .map(CallbackValue::encode_callback)
                .unwrap_or_default()
        }

        fn decode_callback(value: &str) -> Result<Self> {
            if value.is_empty() {
                Ok(None)
            } else {
                T::decode_callback(value).map(Some)
            }
        }
    }

    #[doc(hidden)]
    pub fn pack_parts(prefix: &str, values: &[String]) -> Result<String> {
        pack_parts_with_separator(prefix, ":", values)
    }

    #[doc(hidden)]
    pub fn pack_parts_with_separator(
        prefix: &str,
        separator: &str,
        values: &[String],
    ) -> Result<String> {
        if separator.is_empty() {
            return Err(Error::Utility(
                "callback separator cannot be empty".to_owned(),
            ));
        }
        if prefix.contains(separator) || values.iter().any(|value| value.contains(separator)) {
            return Err(Error::Utility(format!(
                "callback prefix and values cannot contain {separator:?}"
            )));
        }
        let packed = std::iter::once(prefix)
            .chain(values.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(separator);
        if packed.len() > MAX_CALLBACK_LENGTH {
            return Err(Error::Utility(format!(
                "callback data exceeds {MAX_CALLBACK_LENGTH} bytes"
            )));
        }
        Ok(packed)
    }

    #[doc(hidden)]
    pub fn unpack_parts<'a>(
        prefix: &str,
        packed: &'a str,
        expected: usize,
    ) -> Result<Vec<&'a str>> {
        unpack_parts_with_separator(prefix, ":", packed, expected)
    }

    #[doc(hidden)]
    pub fn unpack_parts_with_separator<'a>(
        prefix: &str,
        separator: &str,
        packed: &'a str,
        expected: usize,
    ) -> Result<Vec<&'a str>> {
        if separator.is_empty() {
            return Err(Error::Utility(
                "callback separator cannot be empty".to_owned(),
            ));
        }
        let mut parts = packed.split(separator);
        if parts.next() != Some(prefix) {
            return Err(Error::Utility("callback data prefix mismatch".to_owned()));
        }
        let values: Vec<_> = parts.collect();
        if values.len() != expected {
            return Err(Error::Utility(format!(
                "callback data expected {expected} values, got {}",
                values.len()
            )));
        }
        Ok(values)
    }

    /// Runtime callback-data builder for typed handler protocols.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CallbackData {
        prefix: String,
        separator: String,
        values: Vec<String>,
    }

    impl CallbackData {
        pub fn new(prefix: impl Into<String>) -> Result<Self> {
            Self::with_separator(prefix, ":")
        }

        pub fn with_separator(prefix: impl Into<String>, separator: impl ToString) -> Result<Self> {
            let prefix = prefix.into();
            let separator = separator.to_string();
            if separator.is_empty() {
                return Err(Error::Utility(
                    "callback separator cannot be empty".to_owned(),
                ));
            }
            if prefix.contains(&separator) {
                return Err(Error::Utility(format!(
                    "callback prefix cannot contain separator {separator:?}"
                )));
            }
            Ok(Self {
                prefix,
                separator,
                values: Vec::new(),
            })
        }

        pub fn push(mut self, value: impl Display) -> Result<Self> {
            let value = value.to_string();
            if value.contains(&self.separator) {
                return Err(Error::Utility(format!(
                    "callback value cannot contain separator {:?}",
                    self.separator
                )));
            }
            self.values.push(value);
            Ok(self)
        }

        pub fn push_optional<T: Display>(mut self, value: Option<T>) -> Result<Self> {
            self.values
                .push(value.map(|value| value.to_string()).unwrap_or_default());
            if self
                .values
                .last()
                .is_some_and(|value| value.contains(&self.separator))
            {
                return Err(Error::Utility(format!(
                    "callback value cannot contain separator {:?}",
                    self.separator
                )));
            }
            Ok(self)
        }

        /// Pushes a value using the same encoding rules as typed callback data.
        pub fn push_value<T: CallbackValue>(mut self, value: T) -> Result<Self> {
            let value = value.encode_callback();
            if value.contains(&self.separator) {
                return Err(Error::Utility(format!(
                    "callback value cannot contain separator {:?}",
                    self.separator
                )));
            }
            self.values.push(value);
            Ok(self)
        }

        pub fn pack(&self) -> Result<String> {
            let mut parts = Vec::with_capacity(self.values.len() + 1);
            parts.push(self.prefix.as_str());
            parts.extend(self.values.iter().map(String::as_str));
            let packed = parts.join(&self.separator);
            if packed.len() > MAX_CALLBACK_LENGTH {
                return Err(Error::Utility(format!(
                    "callback data exceeds {MAX_CALLBACK_LENGTH} bytes"
                )));
            }
            Ok(packed)
        }

        pub fn unpack<'a>(&self, packed: &'a str) -> Result<Vec<&'a str>> {
            let mut parts = packed.split(&self.separator);
            if parts.next() != Some(self.prefix.as_str()) {
                return Err(Error::Utility("callback data prefix mismatch".to_owned()));
            }
            let values: Vec<_> = parts.collect();
            if values.len() != self.values.len() {
                return Err(Error::Utility(format!(
                    "callback data expected {} values, got {}",
                    self.values.len(),
                    values.len()
                )));
            }
            Ok(values)
        }

        pub fn prefix(&self) -> &str {
            &self.prefix
        }

        pub fn separator(&self) -> &str {
            &self.separator
        }
    }

    impl PackCallbackData for CallbackData {
        fn pack_callback_data(&self) -> Result<String> {
            self.pack()
        }
    }
}

/// Declares strongly typed callback data with aiogram's compact prefix protocol.
#[macro_export]
macro_rules! callback_data {
    (
        $visibility:vis struct $name:ident($prefix:literal) {
            $($field:ident: $type:ty),+ $(,)?
        }
    ) => {
        $crate::callback_data! {
            @impl $visibility struct $name($prefix, ":") {
                $($field: $type),+
            }
        }
    };
    (
        $visibility:vis struct $name:ident(
            $prefix:literal, separator = $separator:literal
        ) {
            $($field:ident: $type:ty),+ $(,)?
        }
    ) => {
        $crate::callback_data! {
            @impl $visibility struct $name($prefix, $separator) {
                $($field: $type),+
            }
        }
    };
    (
        @impl $visibility:vis struct $name:ident(
            $prefix:literal, $separator:literal
        ) {
            $($field:ident: $type:ty),+ $(,)?
        }
    ) => {
        #[derive(Debug, Clone, PartialEq)]
        $visibility struct $name {
            $($visibility $field: $type),+
        }

        impl $name {
            $visibility fn new($($field: $type),+) -> Self {
                Self { $($field),+ }
            }

            $visibility fn pack(&self) -> $crate::Result<String> {
                $crate::utils::callback_data::pack_parts_with_separator(
                    $prefix,
                    $separator,
                    &[$(<$type as $crate::utils::callback_data::CallbackValue>::encode_callback(
                        &self.$field,
                    )),+],
                )
            }

            $visibility fn unpack(value: &str) -> $crate::Result<Self> {
                let values = $crate::utils::callback_data::unpack_parts_with_separator(
                    $prefix,
                    $separator,
                    value,
                    [$(stringify!($field)),+].len(),
                )?;
                let mut values = values.into_iter();
                Ok(Self {
                    $($field: <$type as $crate::utils::callback_data::CallbackValue>::decode_callback(
                        values
                            .next()
                            .expect("callback value count was validated"),
                    ).map_err(|error| $crate::Error::Utility(format!(
                        "invalid callback field {}: {error}",
                        stringify!($field),
                    )))?),+
                })
            }
        }

        impl $crate::utils::callback_data::PackCallbackData for $name {
            fn pack_callback_data(&self) -> $crate::Result<String> {
                self.pack()
            }
        }
    };
}

/// Builders for Telegram, `tg://`, and aiogram documentation links.
pub mod link {
    use url::Url;

    pub const BASE_DOCS_URL: &str = "https://docs.aiogram.dev/";
    pub const AIOGRAM_BRANCH: &str = "dev-3.x";
    pub const BASE_PAGE_URL: &str = "https://docs.aiogram.dev/en/dev-3.x/";

    fn append_query<K, V>(url: &mut Url, query: impl IntoIterator<Item = (K, V)>)
    where
        K: AsRef<str>,
        V: ToString,
    {
        let mut query = query.into_iter().peekable();
        if query.peek().is_none() {
            return;
        }
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key.as_ref(), &value.to_string());
        }
    }

    /// Corresponds to aiogram's private `_format_url` helper.
    pub fn format_url<K, V>(
        base: &str,
        path: &[&str],
        query: impl IntoIterator<Item = (K, V)>,
        fragment: Option<&str>,
    ) -> String
    where
        K: AsRef<str>,
        V: ToString,
    {
        let joined_path = path.join("/");
        let mut url = if joined_path.is_empty() {
            Url::parse(base).expect("aiogram-rust link helpers use valid constant base URLs")
        } else {
            Url::parse(base)
                .expect("aiogram-rust link helpers use valid constant base URLs")
                .join(&joined_path)
                .expect("Telegram link path must form a valid URL")
        };
        append_query(&mut url, query);
        url.set_fragment(fragment);
        url.into()
    }

    pub fn docs_url(path: &[&str], fragment: Option<&str>) -> String {
        format_url(
            BASE_PAGE_URL,
            path,
            std::iter::empty::<(&str, &str)>(),
            fragment,
        )
    }

    pub fn docs_url_with_query<K, V>(
        path: &[&str],
        query: impl IntoIterator<Item = (K, V)>,
        fragment: Option<&str>,
    ) -> String
    where
        K: AsRef<str>,
        V: ToString,
    {
        format_url(BASE_PAGE_URL, path, query, fragment)
    }

    pub fn create_tg_link<K, V>(link: &str, query: impl IntoIterator<Item = (K, V)>) -> String
    where
        K: AsRef<str>,
        V: ToString,
    {
        format_url(&format!("tg://{link}"), &[], query, None)
    }

    pub fn create_telegram_link(path: &[&str]) -> String {
        format_url(
            "https://t.me",
            path,
            std::iter::empty::<(&str, &str)>(),
            None,
        )
    }

    pub fn create_telegram_link_with_query<K, V>(
        path: &[&str],
        query: impl IntoIterator<Item = (K, V)>,
    ) -> String
    where
        K: AsRef<str>,
        V: ToString,
    {
        format_url("https://t.me", path, query, None)
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct ChannelBotLinkOptions {
        pub parameter: Option<String>,
        pub change_info: bool,
        pub post_messages: bool,
        pub edit_messages: bool,
        pub delete_messages: bool,
        pub restrict_members: bool,
        pub invite_users: bool,
        pub pin_messages: bool,
        pub promote_members: bool,
        pub manage_video_chats: bool,
        pub anonymous: bool,
        pub manage_chat: bool,
    }

    impl ChannelBotLinkOptions {
        pub fn parameter(mut self, value: impl Into<String>) -> Self {
            self.parameter = Some(value.into());
            self
        }

        fn permissions(&self) -> Vec<&'static str> {
            [
                (self.change_info, "change_info"),
                (self.post_messages, "post_messages"),
                (self.edit_messages, "edit_messages"),
                (self.delete_messages, "delete_messages"),
                (self.restrict_members, "restrict_members"),
                (self.invite_users, "invite_users"),
                (self.pin_messages, "pin_messages"),
                (self.promote_members, "promote_members"),
                (self.manage_video_chats, "manage_video_chats"),
                (self.anonymous, "anonymous"),
                (self.manage_chat, "manage_chat"),
            ]
            .into_iter()
            .filter_map(|(enabled, permission)| enabled.then_some(permission))
            .collect()
        }
    }

    pub fn create_channel_bot_link(username: &str) -> String {
        create_telegram_link(&[username])
    }

    pub fn create_channel_bot_link_with_options(
        username: &str,
        options: &ChannelBotLinkOptions,
    ) -> String {
        let mut query = Vec::new();
        if let Some(parameter) = &options.parameter {
            query.push(("startgroup", parameter.clone()));
        }
        let permissions = options.permissions();
        if !permissions.is_empty() {
            query.push(("admin", permissions.join("+")));
        }
        create_telegram_link_with_query(&[username], query)
    }
}

pub mod deep_linking {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use crate::error::{Error, Result};
    use crate::utils::link::create_telegram_link_with_query;

    pub const MAX_PAYLOAD_LENGTH: usize = 64;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DeepLinkType {
        Start,
        StartGroup,
        StartApp,
    }

    impl DeepLinkType {
        fn as_str(self) -> &'static str {
            match self {
                Self::Start => "start",
                Self::StartGroup => "startgroup",
                Self::StartApp => "startapp",
            }
        }
    }

    pub fn encode_payload(payload: impl AsRef<[u8]>) -> String {
        URL_SAFE_NO_PAD.encode(payload.as_ref())
    }

    pub fn decode_payload(payload: &str) -> Result<Vec<u8>> {
        URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|error| Error::Utility(format!("invalid deep-link payload: {error}")))
    }

    pub fn create_deep_link(
        username: &str,
        link_type: DeepLinkType,
        payload: &str,
        encode: bool,
        app_name: Option<&str>,
    ) -> Result<String> {
        let payload = if encode {
            encode_payload(payload)
        } else {
            payload.to_owned()
        };
        create_deep_link_from_payload(username, link_type, payload, app_name)
    }

    /// Creates a deep link after applying a custom binary transform followed
    /// by URL-safe Base64, matching aiogram's `encoder=` workflow.
    pub fn create_deep_link_with_encoder(
        username: &str,
        link_type: DeepLinkType,
        payload: &str,
        app_name: Option<&str>,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_deep_link_from_payload(
            username,
            link_type,
            crate::utils::payload::encode_payload_with(payload, encoder),
            app_name,
        )
    }

    fn create_deep_link_from_payload(
        username: &str,
        link_type: DeepLinkType,
        payload: String,
        app_name: Option<&str>,
    ) -> Result<String> {
        if payload.len() > MAX_PAYLOAD_LENGTH {
            return Err(Error::Utility(format!(
                "deep-link payload exceeds {MAX_PAYLOAD_LENGTH} characters"
            )));
        }
        if !payload
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(Error::Utility(
                "deep-link payload accepts only A-Z, a-z, 0-9, '_' and '-' unless encoded"
                    .to_owned(),
            ));
        }
        let username = username.trim_start_matches('@');
        let path = match app_name {
            Some(app) => vec![username, app],
            None => vec![username],
        };
        Ok(create_telegram_link_with_query(
            &path,
            [(link_type.as_str(), payload)],
        ))
    }

    pub fn create_start_link(username: &str, payload: &str, encode: bool) -> Result<String> {
        create_deep_link(username, DeepLinkType::Start, payload, encode, None)
    }

    pub fn create_start_link_with_encoder(
        username: &str,
        payload: &str,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_deep_link_with_encoder(username, DeepLinkType::Start, payload, None, encoder)
    }

    pub fn create_startgroup_link(username: &str, payload: &str, encode: bool) -> Result<String> {
        create_deep_link(username, DeepLinkType::StartGroup, payload, encode, None)
    }

    pub fn create_startgroup_link_with_encoder(
        username: &str,
        payload: &str,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_deep_link_with_encoder(username, DeepLinkType::StartGroup, payload, None, encoder)
    }

    pub fn create_startapp_link(
        username: &str,
        payload: &str,
        encode: bool,
        app_name: Option<&str>,
    ) -> Result<String> {
        create_deep_link(username, DeepLinkType::StartApp, payload, encode, app_name)
    }

    pub fn create_startapp_link_with_encoder(
        username: &str,
        payload: &str,
        app_name: Option<&str>,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_deep_link_with_encoder(username, DeepLinkType::StartApp, payload, app_name, encoder)
    }

    async fn username(bot: &crate::Bot) -> Result<String> {
        bot.get_me().await?.username.ok_or_else(|| {
            Error::Utility("bot account does not expose a username for deep links".to_owned())
        })
    }

    pub async fn create_start_link_for_bot(
        bot: &crate::Bot,
        payload: &str,
        encode: bool,
    ) -> Result<String> {
        create_start_link(&username(bot).await?, payload, encode)
    }

    pub async fn create_start_link_for_bot_with_encoder(
        bot: &crate::Bot,
        payload: &str,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_start_link_with_encoder(&username(bot).await?, payload, encoder)
    }

    pub async fn create_startgroup_link_for_bot(
        bot: &crate::Bot,
        payload: &str,
        encode: bool,
    ) -> Result<String> {
        create_startgroup_link(&username(bot).await?, payload, encode)
    }

    pub async fn create_startgroup_link_for_bot_with_encoder(
        bot: &crate::Bot,
        payload: &str,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_startgroup_link_with_encoder(&username(bot).await?, payload, encoder)
    }

    pub async fn create_startapp_link_for_bot(
        bot: &crate::Bot,
        payload: &str,
        encode: bool,
        app_name: Option<&str>,
    ) -> Result<String> {
        create_startapp_link(&username(bot).await?, payload, encode, app_name)
    }

    pub async fn create_startapp_link_for_bot_with_encoder(
        bot: &crate::Bot,
        payload: &str,
        app_name: Option<&str>,
        encoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        create_startapp_link_with_encoder(&username(bot).await?, payload, app_name, encoder)
    }
}

pub mod payload {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use crate::error::{Error, Result};

    pub fn encode_payload(payload: &str) -> String {
        URL_SAFE_NO_PAD.encode(payload.as_bytes())
    }

    pub fn encode_payload_with(payload: &str, encoder: impl FnOnce(&[u8]) -> Vec<u8>) -> String {
        URL_SAFE_NO_PAD.encode(encoder(payload.as_bytes()))
    }

    pub fn decode_payload(payload: &str) -> Result<String> {
        decode_payload_with(payload, |value| value.to_vec())
    }

    pub fn decode_payload_with(
        payload: &str,
        decoder: impl FnOnce(&[u8]) -> Vec<u8>,
    ) -> Result<String> {
        let decoded = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|error| Error::Utility(format!("invalid base64url payload: {error}")))?;
        String::from_utf8(decoder(&decoded))
            .map_err(|error| Error::Utility(format!("payload is not UTF-8: {error}")))
    }
}

pub mod serialization {
    use serde::Serialize;

    use crate::methods::TelegramMethod;
    use crate::types::{CollectFiles, InputFileUpload};

    #[derive(Debug, Clone)]
    pub struct DeserializedTelegramObject {
        pub data: serde_json::Value,
        pub files: Vec<InputFileUpload>,
    }

    pub fn deserialize_telegram_object<T>(
        object: &T,
    ) -> serde_json::Result<DeserializedTelegramObject>
    where
        T: Serialize + CollectFiles,
    {
        let mut files = Vec::new();
        object.collect_files(&mut files);
        Ok(DeserializedTelegramObject {
            data: serde_json::to_value(object)?,
            files,
        })
    }

    pub fn deserialize_method<M: TelegramMethod>(
        method: &M,
        include_api_method_name: bool,
    ) -> serde_json::Result<DeserializedTelegramObject> {
        let mut object = deserialize_telegram_object(method)?;
        if include_api_method_name && let Some(data) = object.data.as_object_mut() {
            data.insert(
                "method".to_owned(),
                serde_json::Value::String(M::NAME.to_owned()),
            );
        }
        Ok(object)
    }

    pub fn deserialize_telegram_object_to_value<T>(
        object: &T,
    ) -> serde_json::Result<serde_json::Value>
    where
        T: Serialize + CollectFiles,
    {
        deserialize_telegram_object(object).map(|object| object.data)
    }
}

pub mod formatting {
    use std::cmp::Reverse;
    use std::ops::Range;

    use crate::error::{Error, Result};
    use crate::methods::SendMessage;
    use crate::types::{ChatId, MessageEntity, User};

    /// Returns Telegram's UTF-16 code-unit length for entity offsets.
    pub fn sizeof(value: &str) -> usize {
        value.encode_utf16().count()
    }

    /// Rendered Telegram text with UTF-16-based entities.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RenderedText {
        pub text: String,
        pub entities: Vec<MessageEntity>,
    }

    impl RenderedText {
        pub fn into_fields(
            self,
            text_key: &str,
            entities_key: &str,
            parse_mode_key: &str,
            replace_parse_mode: bool,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            let mut fields = serde_json::Map::new();
            fields.insert(text_key.to_owned(), serde_json::Value::String(self.text));
            fields.insert(
                entities_key.to_owned(),
                serde_json::to_value(self.entities)?,
            );
            if replace_parse_mode {
                fields.insert(parse_mode_key.to_owned(), serde_json::Value::Null);
            }
            Ok(fields)
        }

        pub fn into_send_message(self, chat_id: impl Into<ChatId>) -> SendMessage {
            let mut method = SendMessage::new(chat_id, self.text).entities(self.entities);
            method
                .extra
                .insert("parse_mode".to_owned(), serde_json::Value::Null);
            method
        }

        pub fn as_html(&self) -> Result<String> {
            html_text(&self.text, &self.entities)
        }

        pub fn as_markdown(&self) -> Result<String> {
            markdown_text(&self.text, &self.entities)
        }
    }

    #[derive(Debug, Clone, Default, PartialEq)]
    struct EntityParams {
        url: Option<String>,
        user: Option<User>,
        language: Option<String>,
        custom_emoji_id: Option<String>,
        unix_time: Option<i64>,
        date_time_format: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq)]
    enum TextNode {
        Plain(String),
        Entity {
            kind: String,
            params: Box<EntityParams>,
            body: Vec<TextNode>,
        },
    }

    /// Composable entity-based text, corresponding to `aiogram.utils.formatting.Text`.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct Text {
        body: Vec<TextNode>,
    }

    /// A Bot API method that accepts `caption` and `caption_entities`.
    ///
    /// Applying formatted text also inserts the internal explicit-null marker
    /// used by [`crate::Bot`] to suppress a configured default parse mode.
    pub trait WithFormattedCaption: Sized {
        fn with_formatted_caption(self, rendered: RenderedText) -> Self;
    }

    pub trait WithFormattedText: Sized {
        fn with_formatted_text(self, rendered: RenderedText) -> Self;
    }

    pub trait WithFormattedGiftText: Sized {
        fn with_formatted_gift_text(self, rendered: RenderedText) -> Self;
    }

    pub trait WithFormattedPollQuestion: Sized {
        fn with_formatted_poll_question(self, rendered: RenderedText) -> Self;
    }

    pub trait WithFormattedPollExplanation: Sized {
        fn with_formatted_poll_explanation(self, rendered: RenderedText) -> Self;
    }

    impl Text {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn plain(value: impl Into<String>) -> Self {
            Self {
                body: vec![TextNode::Plain(value.into())],
            }
        }

        /// Reconstructs a composable tree from Telegram's UTF-16 entity offsets.
        pub fn from_entities(text: &str, entities: &[MessageEntity]) -> Result<Self> {
            let boundaries = utf16_boundaries(text);
            let total = boundaries.len().saturating_sub(1);
            let mut spans = Vec::with_capacity(entities.len());
            for entity in entities {
                let start = usize::try_from(entity.offset).map_err(|_| {
                    Error::Utility("message entity offset cannot be negative".to_owned())
                })?;
                let length = usize::try_from(entity.length).map_err(|_| {
                    Error::Utility("message entity length cannot be negative".to_owned())
                })?;
                let end = start.checked_add(length).ok_or_else(|| {
                    Error::Utility("message entity UTF-16 range overflowed".to_owned())
                })?;
                if end > total
                    || boundaries.get(start).copied().flatten().is_none()
                    || boundaries.get(end).copied().flatten().is_none()
                {
                    return Err(Error::Utility(format!(
                        "message entity {}..{} is not a valid UTF-16 text range",
                        start, end
                    )));
                }
                spans.push(EntitySpan { entity, start, end });
            }
            spans.sort_by_key(|span| (span.start, Reverse(span.end)));
            Ok(Self {
                body: parse_entity_nodes(text, &boundaries, &spans, 0, total)?,
            })
        }

        pub fn concat(items: impl IntoIterator<Item = impl Into<Text>>) -> Self {
            let mut text = Self::new();
            for item in items {
                text.body.extend(item.into().body);
            }
            text
        }

        pub fn then(mut self, value: impl Into<Text>) -> Self {
            self.body.extend(value.into().body);
            self
        }

        /// Replaces the root body while preserving a root entity's type and
        /// parameters, matching `Text.replace(...)` in aiogram.
        pub fn replace(&self, items: impl IntoIterator<Item = impl Into<Text>>) -> Self {
            let replacement = Self::concat(items).body;
            match self.body.as_slice() {
                [TextNode::Entity { kind, params, .. }] => Self {
                    body: vec![TextNode::Entity {
                        kind: kind.clone(),
                        params: params.clone(),
                        body: replacement,
                    }],
                },
                _ => Self { body: replacement },
            }
        }

        pub fn entity(kind: impl Into<String>, body: impl Into<Text>) -> Self {
            Self::entity_with(kind, body.into(), EntityParams::default())
        }

        fn entity_with(kind: impl Into<String>, body: Text, params: EntityParams) -> Self {
            Self {
                body: vec![TextNode::Entity {
                    kind: kind.into(),
                    params: Box::new(params),
                    body: body.body,
                }],
            }
        }

        pub fn utf16_len(&self) -> usize {
            let mut text = String::new();
            render_nodes(&self.body, &mut text, &mut Vec::new(), false);
            text.encode_utf16().count()
        }

        /// Returns a UTF-16 range while preserving and clipping nested entities.
        ///
        /// Telegram expresses every entity offset in UTF-16 code units. Using
        /// the same coordinate system here avoids corrupting astral Unicode
        /// characters when a formatted fragment is extracted.
        pub fn slice_utf16(&self, range: Range<usize>) -> Result<Self> {
            if range.start > range.end {
                return Err(Error::Utility(
                    "formatted text slice start cannot exceed its end".to_owned(),
                ));
            }
            let rendered = self.render();
            let boundaries = utf16_boundaries(&rendered.text);
            let total = boundaries.len().saturating_sub(1);
            if range.end > total {
                return Err(Error::Utility(format!(
                    "formatted text slice {}..{} exceeds UTF-16 length {total}",
                    range.start, range.end
                )));
            }
            let text = text_range(&rendered.text, &boundaries, range.start, range.end)?;
            let mut entities = Vec::new();
            for mut entity in rendered.entities {
                let entity_start = usize::try_from(entity.offset).map_err(|_| {
                    Error::Utility("message entity offset cannot be negative".to_owned())
                })?;
                let entity_length = usize::try_from(entity.length).map_err(|_| {
                    Error::Utility("message entity length cannot be negative".to_owned())
                })?;
                let entity_end = entity_start.checked_add(entity_length).ok_or_else(|| {
                    Error::Utility("message entity UTF-16 range overflowed".to_owned())
                })?;
                let start = entity_start.max(range.start);
                let end = entity_end.min(range.end);
                if start < end {
                    entity.offset = i64::try_from(start - range.start).map_err(|_| {
                        Error::Utility("message entity offset exceeds i64".to_owned())
                    })?;
                    entity.length = i64::try_from(end - start).map_err(|_| {
                        Error::Utility("message entity length exceeds i64".to_owned())
                    })?;
                    entities.push(entity);
                }
            }
            Self::from_entities(&text, &entities)
        }

        pub fn render(&self) -> RenderedText {
            let mut text = String::new();
            let mut entities = Vec::new();
            render_nodes(&self.body, &mut text, &mut entities, true);
            entities.sort_by_key(|entity| entity.offset);
            RenderedText { text, entities }
        }

        pub fn as_fields(
            &self,
            text_key: &str,
            entities_key: &str,
            parse_mode_key: &str,
            replace_parse_mode: bool,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.render()
                .into_fields(text_key, entities_key, parse_mode_key, replace_parse_mode)
        }

        pub fn as_kwargs(&self) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.as_fields("text", "entities", "parse_mode", true)
        }

        pub fn as_caption_kwargs(
            &self,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.as_fields("caption", "caption_entities", "parse_mode", true)
        }

        pub fn as_poll_question_kwargs(
            &self,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.as_fields("question", "question_entities", "question_parse_mode", true)
        }

        pub fn as_poll_explanation_kwargs(
            &self,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.as_fields(
                "explanation",
                "explanation_entities",
                "explanation_parse_mode",
                true,
            )
        }

        pub fn as_gift_text_kwargs(
            &self,
        ) -> serde_json::Result<serde_json::Map<String, serde_json::Value>> {
            self.as_fields("text", "text_entities", "text_parse_mode", true)
        }

        pub fn as_pretty_string(&self, indent: bool) -> String {
            if indent {
                format!("{self:#?}")
            } else {
                format!("{self:?}")
            }
        }

        pub fn into_send_message(self, chat_id: impl Into<ChatId>) -> SendMessage {
            self.render().into_send_message(chat_id)
        }

        pub fn apply_caption<T: WithFormattedCaption>(&self, target: T) -> T {
            target.with_formatted_caption(self.render())
        }

        pub fn apply_text<T: WithFormattedText>(&self, target: T) -> T {
            target.with_formatted_text(self.render())
        }

        pub fn apply_gift_text<T: WithFormattedGiftText>(&self, target: T) -> T {
            target.with_formatted_gift_text(self.render())
        }

        pub fn apply_poll_question<T: WithFormattedPollQuestion>(&self, target: T) -> T {
            target.with_formatted_poll_question(self.render())
        }

        pub fn apply_poll_explanation<T: WithFormattedPollExplanation>(&self, target: T) -> T {
            target.with_formatted_poll_explanation(self.render())
        }

        pub fn as_html(&self) -> String {
            decorate_nodes(&self.body, Decoration::Html)
        }

        pub fn as_markdown(&self) -> String {
            decorate_nodes(&self.body, Decoration::MarkdownV2)
        }
    }

    impl From<String> for Text {
        fn from(value: String) -> Self {
            Self::plain(value)
        }
    }

    impl From<&str> for Text {
        fn from(value: &str) -> Self {
            Self::plain(value)
        }
    }

    macro_rules! formatted_caption_targets {
        ($($target:ty),+ $(,)?) => {
            $(
                impl WithFormattedCaption for $target {
                    fn with_formatted_caption(mut self, rendered: RenderedText) -> Self {
                        self.caption = Some(rendered.text);
                        self.caption_entities = Some(rendered.entities);
                        self.parse_mode = None;
                        self.extra.insert(
                            "parse_mode".to_owned(),
                            serde_json::Value::Null,
                        );
                        self
                    }
                }
            )+
        };
    }

    formatted_caption_targets!(
        crate::methods::CopyMessage,
        crate::methods::EditEphemeralMessageCaption,
        crate::methods::EditMessageCaption,
        crate::methods::EditStory,
        crate::methods::PostStory,
        crate::methods::SendAnimation,
        crate::methods::SendAudio,
        crate::methods::SendDocument,
        crate::methods::SendLivePhoto,
        crate::methods::SendPaidMedia,
        crate::methods::SendPhoto,
        crate::methods::SendVideo,
        crate::methods::SendVoice,
    );

    macro_rules! formatted_required_text_targets {
        ($($target:ty),+ $(,)?) => {
            $(
                impl WithFormattedText for $target {
                    fn with_formatted_text(mut self, rendered: RenderedText) -> Self {
                        self.text = rendered.text;
                        self.entities = Some(rendered.entities);
                        self.parse_mode = None;
                        self.extra.insert(
                            "parse_mode".to_owned(),
                            serde_json::Value::Null,
                        );
                        self
                    }
                }
            )+
        };
    }

    formatted_required_text_targets!(
        crate::methods::EditEphemeralMessageText,
        crate::methods::SendMessage,
    );

    macro_rules! formatted_optional_text_targets {
        ($($target:ty),+ $(,)?) => {
            $(
                impl WithFormattedText for $target {
                    fn with_formatted_text(mut self, rendered: RenderedText) -> Self {
                        self.text = Some(rendered.text);
                        self.entities = Some(rendered.entities);
                        self.parse_mode = None;
                        self.extra.insert(
                            "parse_mode".to_owned(),
                            serde_json::Value::Null,
                        );
                        self
                    }
                }
            )+
        };
    }

    formatted_optional_text_targets!(
        crate::methods::EditMessageText,
        crate::methods::SendMessageDraft,
    );

    macro_rules! formatted_gift_text_targets {
        ($($target:ty),+ $(,)?) => {
            $(
                impl WithFormattedGiftText for $target {
                    fn with_formatted_gift_text(self, rendered: RenderedText) -> Self {
                        let mut target = self.text(rendered.text);
                        target.text_entities = Some(rendered.entities);
                        target.text_parse_mode = None;
                        target
                    }
                }
            )+
        };
    }

    formatted_gift_text_targets!(
        crate::methods::GiftPremiumSubscription,
        crate::methods::SendGift,
    );

    impl WithFormattedPollQuestion for crate::methods::SendPoll {
        fn with_formatted_poll_question(mut self, rendered: RenderedText) -> Self {
            self.question = rendered.text;
            self.question_entities = Some(rendered.entities);
            self.question_parse_mode = None;
            self.extra
                .insert("question_parse_mode".to_owned(), serde_json::Value::Null);
            self
        }
    }

    impl WithFormattedPollExplanation for crate::methods::SendPoll {
        fn with_formatted_poll_explanation(mut self, rendered: RenderedText) -> Self {
            self.explanation = Some(rendered.text);
            self.explanation_entities = Some(rendered.entities);
            self.explanation_parse_mode = None;
            self.extra
                .insert("explanation_parse_mode".to_owned(), serde_json::Value::Null);
            self
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct EntitySpan<'a> {
        entity: &'a MessageEntity,
        start: usize,
        end: usize,
    }

    fn utf16_boundaries(text: &str) -> Vec<Option<usize>> {
        let total = text.encode_utf16().count();
        let mut boundaries = vec![None; total + 1];
        let mut utf16_offset = 0;
        boundaries[0] = Some(0);
        for (byte_offset, character) in text.char_indices() {
            boundaries[utf16_offset] = Some(byte_offset);
            utf16_offset += character.len_utf16();
            boundaries[utf16_offset] = Some(byte_offset + character.len_utf8());
        }
        boundaries
    }

    fn text_range(
        text: &str,
        boundaries: &[Option<usize>],
        start: usize,
        end: usize,
    ) -> Result<String> {
        let start = boundaries.get(start).copied().flatten().ok_or_else(|| {
            Error::Utility("entity starts inside a UTF-16 surrogate pair".to_owned())
        })?;
        let end = boundaries.get(end).copied().flatten().ok_or_else(|| {
            Error::Utility("entity ends inside a UTF-16 surrogate pair".to_owned())
        })?;
        Ok(text[start..end].to_owned())
    }

    fn parse_entity_nodes(
        text: &str,
        boundaries: &[Option<usize>],
        spans: &[EntitySpan<'_>],
        range_start: usize,
        range_end: usize,
    ) -> Result<Vec<TextNode>> {
        let mut nodes = Vec::new();
        let mut cursor = range_start;
        let mut index = 0;
        while index < spans.len() {
            let span = spans[index];
            if span.start < cursor {
                index += 1;
                continue;
            }
            if span.start > range_end || span.end > range_end {
                return Err(Error::Utility(
                    "message entities contain crossing ranges".to_owned(),
                ));
            }
            if span.start > cursor {
                nodes.push(TextNode::Plain(text_range(
                    text, boundaries, cursor, span.start,
                )?));
            }

            let mut nested_end = index + 1;
            while nested_end < spans.len() && spans[nested_end].start < span.end {
                if spans[nested_end].end > span.end {
                    return Err(Error::Utility(
                        "message entities contain crossing ranges".to_owned(),
                    ));
                }
                nested_end += 1;
            }
            let body = parse_entity_nodes(
                text,
                boundaries,
                &spans[index + 1..nested_end],
                span.start,
                span.end,
            )?;
            nodes.push(TextNode::Entity {
                kind: span.entity.kind.clone(),
                params: Box::new(EntityParams {
                    url: span.entity.url.clone(),
                    user: span.entity.user.clone(),
                    language: span.entity.language.clone(),
                    custom_emoji_id: span.entity.custom_emoji_id.clone(),
                    unix_time: span.entity.unix_time,
                    date_time_format: span.entity.date_time_format.clone(),
                }),
                body,
            });
            cursor = span.end;
            index = nested_end;
        }
        if cursor < range_end {
            nodes.push(TextNode::Plain(text_range(
                text, boundaries, cursor, range_end,
            )?));
        }
        Ok(nodes)
    }

    #[derive(Debug, Clone, Copy)]
    enum Decoration {
        Html,
        MarkdownV2,
    }

    fn decorate_nodes(nodes: &[TextNode], mode: Decoration) -> String {
        let mut output = String::new();
        for node in nodes {
            match node {
                TextNode::Plain(value) => match mode {
                    Decoration::Html => output.push_str(&html_text_quote(value)),
                    Decoration::MarkdownV2 => output.push_str(&markdown_v2_quote(value)),
                },
                TextNode::Entity { kind, params, body } => {
                    let body = decorate_nodes(body, mode);
                    output.push_str(&decorate_entity(kind, params, &body, mode));
                }
            }
        }
        output
    }

    fn decorate_entity(kind: &str, params: &EntityParams, body: &str, mode: Decoration) -> String {
        match mode {
            Decoration::Html => match kind {
                "bold" => format!("<b>{body}</b>"),
                "italic" => format!("<i>{body}</i>"),
                "underline" => format!("<u>{body}</u>"),
                "strikethrough" => format!("<s>{body}</s>"),
                "spoiler" => format!("<tg-spoiler>{body}</tg-spoiler>"),
                "code" => format!("<code>{body}</code>"),
                "pre" => match params.language.as_deref() {
                    Some(language) => format!(
                        "<pre><code language=\"language-{}\">{body}</code></pre>",
                        html_attribute(language)
                    ),
                    None => format!("<pre>{body}</pre>"),
                },
                "text_link" => params.url.as_deref().map_or_else(
                    || body.to_owned(),
                    |url| format!("<a href=\"{}\">{body}</a>", html_attribute(url)),
                ),
                "text_mention" => params.user.as_ref().map_or_else(
                    || body.to_owned(),
                    |user| format!("<a href=\"tg://user?id={}\">{body}</a>", user.id),
                ),
                "custom_emoji" => params.custom_emoji_id.as_deref().map_or_else(
                    || body.to_owned(),
                    |id| {
                        format!(
                            "<tg-emoji emoji-id=\"{}\">{body}</tg-emoji>",
                            html_attribute(id)
                        )
                    },
                ),
                "date_time" => params.unix_time.map_or_else(
                    || body.to_owned(),
                    |unix| {
                        let format = params
                            .date_time_format
                            .as_deref()
                            .map_or_else(String::new, |value| {
                                format!(" format=\"{}\"", html_attribute(value))
                            });
                        format!("<tg-time unix=\"{unix}\"{format}>{body}</tg-time>")
                    },
                ),
                "blockquote" => format!("<blockquote>{body}</blockquote>"),
                "expandable_blockquote" => format!("<blockquote expandable>{body}</blockquote>"),
                _ => body.to_owned(),
            },
            Decoration::MarkdownV2 => match kind {
                "bold" => format!("*{body}*"),
                "italic" => format!("_\r{body}_\r"),
                "underline" => format!("__\r{body}__\r"),
                "strikethrough" => format!("~{body}~"),
                "spoiler" => format!("||{body}||"),
                "code" => format!("`{body}`"),
                "pre" => match params.language.as_deref() {
                    Some(language) => format!("```{language}\n{body}\n```"),
                    None => format!("```\n{body}\n```"),
                },
                "text_link" => params
                    .url
                    .as_deref()
                    .map_or_else(|| body.to_owned(), |url| format!("[{body}]({url})")),
                "text_mention" => params.user.as_ref().map_or_else(
                    || body.to_owned(),
                    |user| format!("[{body}](tg://user?id={})", user.id),
                ),
                "custom_emoji" => params.custom_emoji_id.as_deref().map_or_else(
                    || body.to_owned(),
                    |id| format!("![{body}](tg://emoji?emoji_id={id})"),
                ),
                "date_time" => params.unix_time.map_or_else(
                    || body.to_owned(),
                    |unix| {
                        let format = params
                            .date_time_format
                            .as_deref()
                            .map_or_else(String::new, |value| format!("&format={value}"));
                        format!("![{body}](tg://time?unix={unix}{format})")
                    },
                ),
                "blockquote" => body
                    .lines()
                    .map(|line| format!(">{line}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                "expandable_blockquote" => format!(
                    "{}||",
                    body.lines()
                        .map(|line| format!(">{line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
                _ => body.to_owned(),
            },
        }
    }

    fn html_text_quote(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn html_attribute(value: &str) -> String {
        html_text_quote(value).replace('"', "&quot;")
    }

    pub fn html_text(text: &str, entities: &[MessageEntity]) -> Result<String> {
        Ok(Text::from_entities(text, entities)?.as_html())
    }

    pub fn markdown_text(text: &str, entities: &[MessageEntity]) -> Result<String> {
        Ok(Text::from_entities(text, entities)?.as_markdown())
    }

    pub fn extract_entity_text(text: &str, entity: &MessageEntity) -> Result<String> {
        let boundaries = utf16_boundaries(text);
        let start = usize::try_from(entity.offset)
            .map_err(|_| Error::Utility("message entity offset cannot be negative".to_owned()))?;
        let length = usize::try_from(entity.length)
            .map_err(|_| Error::Utility("message entity length cannot be negative".to_owned()))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| Error::Utility("message entity UTF-16 range overflowed".to_owned()))?;
        text_range(text, &boundaries, start, end)
    }

    fn render_nodes(
        nodes: &[TextNode],
        text: &mut String,
        entities: &mut Vec<MessageEntity>,
        collect_entities: bool,
    ) {
        for node in nodes {
            match node {
                TextNode::Plain(value) => text.push_str(value),
                TextNode::Entity { kind, params, body } => {
                    let offset = text.encode_utf16().count() as i64;
                    render_nodes(body, text, entities, collect_entities);
                    if !collect_entities {
                        continue;
                    }
                    let length = text.encode_utf16().count() as i64 - offset;
                    let mut entity = MessageEntity::new(kind.clone(), offset, length);
                    if let Some(url) = &params.url {
                        entity = entity.url(url.clone());
                    }
                    if let Some(user) = &params.user {
                        entity = entity.user(user.clone());
                    }
                    if let Some(language) = &params.language {
                        entity = entity.language(language.clone());
                    }
                    if let Some(custom_emoji_id) = &params.custom_emoji_id {
                        entity = entity.custom_emoji_id(custom_emoji_id.clone());
                    }
                    if let Some(unix_time) = params.unix_time {
                        entity = entity.unix_time(unix_time);
                    }
                    if let Some(date_time_format) = &params.date_time_format {
                        entity = entity.date_time_format(date_time_format.clone());
                    }
                    entities.push(entity);
                }
            }
        }
    }

    macro_rules! simple_entity {
        ($name:ident, $kind:literal) => {
            pub fn $name(body: impl Into<Text>) -> Text {
                Text::entity($kind, body)
            }
        };
    }

    simple_entity!(bold, "bold");
    simple_entity!(italic, "italic");
    simple_entity!(underline, "underline");
    simple_entity!(strikethrough, "strikethrough");
    simple_entity!(spoiler, "spoiler");
    simple_entity!(code, "code");
    simple_entity!(blockquote, "blockquote");
    simple_entity!(expandable_blockquote, "expandable_blockquote");
    simple_entity!(bot_command, "bot_command");
    simple_entity!(url, "url");
    simple_entity!(email, "email");
    simple_entity!(phone_number, "phone_number");

    pub fn hashtag(value: impl Into<String>) -> Text {
        let value = value.into();
        Text::entity(
            "hashtag",
            if value.starts_with('#') {
                value
            } else {
                format!("#{value}")
            },
        )
    }

    pub fn cashtag(value: impl Into<String>) -> Text {
        let value = value.into();
        Text::entity(
            "cashtag",
            if value.starts_with('$') {
                value
            } else {
                format!("${value}")
            },
        )
    }

    pub fn pre(body: impl Into<Text>, language: Option<impl Into<String>>) -> Text {
        Text::entity_with(
            "pre",
            body.into(),
            EntityParams {
                language: language.map(Into::into),
                ..EntityParams::default()
            },
        )
    }

    pub fn text_link(body: impl Into<Text>, url: impl Into<String>) -> Text {
        Text::entity_with(
            "text_link",
            body.into(),
            EntityParams {
                url: Some(url.into()),
                ..EntityParams::default()
            },
        )
    }

    pub fn text_mention(body: impl Into<Text>, user: User) -> Text {
        Text::entity_with(
            "text_mention",
            body.into(),
            EntityParams {
                user: Some(user),
                ..EntityParams::default()
            },
        )
    }

    pub fn custom_emoji(body: impl Into<Text>, custom_emoji_id: impl Into<String>) -> Text {
        Text::entity_with(
            "custom_emoji",
            body.into(),
            EntityParams {
                custom_emoji_id: Some(custom_emoji_id.into()),
                ..EntityParams::default()
            },
        )
    }

    pub fn date_time(
        body: impl Into<Text>,
        unix_time: i64,
        date_time_format: Option<impl Into<String>>,
    ) -> Text {
        Text::entity_with(
            "date_time",
            body.into(),
            EntityParams {
                unix_time: Some(unix_time),
                date_time_format: date_time_format.map(Into::into),
                ..EntityParams::default()
            },
        )
    }

    pub fn as_line(items: impl IntoIterator<Item = impl Into<Text>>) -> Text {
        as_line_with(items, "\n", "")
    }

    pub fn as_line_with(
        items: impl IntoIterator<Item = impl Into<Text>>,
        end: &str,
        separator: &str,
    ) -> Text {
        let mut output = Text::new();
        for (index, item) in items.into_iter().enumerate() {
            if index > 0 && !separator.is_empty() {
                output = output.then(separator);
            }
            output = output.then(item);
        }
        output.then(end)
    }

    pub fn as_list(items: impl IntoIterator<Item = impl Into<Text>>) -> Text {
        as_list_with_separator(items, "\n")
    }

    pub fn as_list_with_separator(
        items: impl IntoIterator<Item = impl Into<Text>>,
        separator: &str,
    ) -> Text {
        let mut output = Text::new();
        for (index, item) in items.into_iter().enumerate() {
            if index > 0 {
                output = output.then(separator);
            }
            output = output.then(item);
        }
        output
    }

    pub fn as_marked_list(items: impl IntoIterator<Item = impl Into<Text>>) -> Text {
        as_marked_list_with_marker(items, "- ")
    }

    pub fn as_marked_list_with_marker(
        items: impl IntoIterator<Item = impl Into<Text>>,
        marker: &str,
    ) -> Text {
        as_list(items.into_iter().map(|item| Text::plain(marker).then(item)))
    }

    pub fn as_numbered_list(
        items: impl IntoIterator<Item = impl Into<Text>>,
        start: usize,
    ) -> Text {
        as_numbered_list_with(items, start, |index| format!("{index}. "))
    }

    pub fn as_numbered_list_with(
        items: impl IntoIterator<Item = impl Into<Text>>,
        start: usize,
        format_marker: impl Fn(usize) -> String,
    ) -> Text {
        as_list(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| Text::plain(format_marker(start + index)).then(item)),
        )
    }

    pub fn as_section(title: impl Into<Text>, body: impl Into<Text>) -> Text {
        title.into().then("\n").then(body)
    }

    pub fn as_marked_section(
        title: impl Into<Text>,
        body: impl IntoIterator<Item = impl Into<Text>>,
    ) -> Text {
        as_section(title, as_marked_list(body))
    }

    pub fn as_numbered_section(
        title: impl Into<Text>,
        body: impl IntoIterator<Item = impl Into<Text>>,
        start: usize,
    ) -> Text {
        as_section(title, as_numbered_list(body, start))
    }

    pub fn as_key_value(key: impl Into<Text>, value: impl Into<Text>) -> Text {
        bold(key.into().then(":")).then(" ").then(value.into())
    }

    pub fn html_quote(value: &str) -> String {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    fn html_attribute_quote(value: &str) -> String {
        html_quote(value).replace('"', "&quot;")
    }

    pub fn html_bold(value: &str) -> String {
        format!("<b>{}</b>", html_quote(value))
    }

    pub fn html_italic(value: &str) -> String {
        format!("<i>{}</i>", html_quote(value))
    }

    pub fn html_code(value: &str) -> String {
        format!("<code>{}</code>", html_quote(value))
    }

    pub fn html_pre(value: &str) -> String {
        format!("<pre>{}</pre>", html_quote(value))
    }

    pub fn html_pre_language(value: &str, language: &str) -> String {
        format!(
            "<pre><code language=\"language-{}\">{}</code></pre>",
            html_attribute_quote(language),
            html_quote(value)
        )
    }

    pub fn html_underline(value: &str) -> String {
        format!("<u>{}</u>", html_quote(value))
    }

    pub fn html_strikethrough(value: &str) -> String {
        format!("<s>{}</s>", html_quote(value))
    }

    pub fn html_blockquote(value: &str) -> String {
        format!("<blockquote>{}</blockquote>", html_quote(value))
    }

    pub fn html_expandable_blockquote(value: &str) -> String {
        format!("<blockquote expandable>{}</blockquote>", html_quote(value))
    }

    pub fn html_spoiler(value: &str) -> String {
        format!("<tg-spoiler>{}</tg-spoiler>", html_quote(value))
    }

    pub fn html_custom_emoji(value: &str, custom_emoji_id: &str) -> String {
        format!(
            "<tg-emoji emoji-id=\"{}\">{}</tg-emoji>",
            html_attribute_quote(custom_emoji_id),
            html_quote(value)
        )
    }

    pub fn html_date_time(value: &str, unix_time: i64, date_time_format: Option<&str>) -> String {
        let date_time_format = date_time_format
            .map(|format| format!(" format=\"{}\"", html_attribute_quote(format)))
            .unwrap_or_default();
        format!(
            "<tg-time unix=\"{unix_time}\"{date_time_format}>{}</tg-time>",
            html_quote(value)
        )
    }

    pub fn html_link(label: &str, url: &str) -> String {
        format!(
            "<a href=\"{}\">{}</a>",
            html_attribute_quote(url),
            html_quote(label)
        )
    }

    pub fn markdown_v2_quote(value: &str) -> String {
        const SPECIAL: &str = "_*[]()~`>#+-=|{}.!\\";
        let mut result = String::with_capacity(value.len());
        for character in value.chars() {
            if SPECIAL.contains(character) {
                result.push('\\');
            }
            result.push(character);
        }
        result
    }

    pub fn markdown_v2_bold(value: &str) -> String {
        format!("*{}*", markdown_v2_quote(value))
    }

    pub fn markdown_v2_italic(value: &str) -> String {
        format!("_\r{}_\r", markdown_v2_quote(value))
    }

    pub fn markdown_v2_code(value: &str) -> String {
        format!("`{}`", markdown_v2_quote(value))
    }

    pub fn markdown_v2_pre(value: &str) -> String {
        format!("```\n{}\n```", markdown_v2_quote(value))
    }

    pub fn markdown_v2_pre_language(value: &str, language: &str) -> String {
        format!(
            "```{}\n{}\n```",
            markdown_v2_quote(language),
            markdown_v2_quote(value)
        )
    }

    pub fn markdown_v2_underline(value: &str) -> String {
        format!("__\r{}__\r", markdown_v2_quote(value))
    }

    pub fn markdown_v2_strikethrough(value: &str) -> String {
        format!("~{}~", markdown_v2_quote(value))
    }

    pub fn markdown_v2_link(label: &str, url: &str) -> String {
        format!("[{}]({url})", markdown_v2_quote(label))
    }

    pub fn markdown_v2_blockquote(value: &str) -> String {
        markdown_v2_quote(value)
            .lines()
            .map(|line| format!(">{line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn markdown_v2_expandable_blockquote(value: &str) -> String {
        format!("{}||", markdown_v2_blockquote(value))
    }

    pub fn markdown_v2_spoiler(value: &str) -> String {
        format!("||{}||", markdown_v2_quote(value))
    }

    pub fn markdown_v2_custom_emoji(value: &str, custom_emoji_id: &str) -> String {
        let url =
            crate::utils::link::create_tg_link("emoji", [("emoji_id", custom_emoji_id.to_owned())]);
        format!("!{}", markdown_v2_link(value, &url))
    }

    pub fn markdown_v2_date_time(
        value: &str,
        unix_time: i64,
        date_time_format: Option<&str>,
    ) -> String {
        let mut query = vec![("unix", unix_time.to_string())];
        if let Some(format) = date_time_format {
            query.push(("format", format.to_owned()));
        }
        let url = crate::utils::link::create_tg_link("time", query);
        format!("!{}", markdown_v2_link(value, &url))
    }

    pub fn hide_link(url: &str) -> String {
        format!("<a href=\"{}\">&#8203;</a>", html_attribute_quote(url))
    }
}

pub mod keyboard {
    use crate::error::{Error, Result};
    use crate::types::{
        InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, ReplyKeyboardMarkup,
    };
    use crate::utils::callback_data::PackCallbackData;

    fn adjusted<T: Clone>(
        buttons: &[T],
        sizes: &[usize],
        repeat: bool,
        max_width: usize,
    ) -> Result<Vec<Vec<T>>> {
        let sizes = if sizes.is_empty() {
            vec![max_width]
        } else {
            sizes.to_vec()
        };
        if sizes.iter().any(|size| *size == 0 || *size > max_width) {
            return Err(Error::Utility(format!(
                "row width must be between 1 and {max_width}"
            )));
        }
        let mut rows = Vec::new();
        let mut offset = 0;
        let mut size_index = 0;
        while offset < buttons.len() {
            let size = sizes[size_index];
            let end = (offset + size).min(buttons.len());
            rows.push(buttons[offset..end].to_vec());
            offset = end;
            if repeat {
                size_index = (size_index + 1) % sizes.len();
            } else if size_index + 1 < sizes.len() {
                size_index += 1;
            }
        }
        Ok(rows)
    }

    #[derive(Debug, Clone, Default)]
    pub struct InlineKeyboardBuilder {
        rows: Vec<Vec<InlineKeyboardButton>>,
    }

    impl InlineKeyboardBuilder {
        pub const MAX_WIDTH: usize = 8;
        pub const MAX_BUTTONS: usize = 100;

        pub fn new() -> Self {
            Self::default()
        }

        pub fn from_markup(markup: InlineKeyboardMarkup) -> Result<Self> {
            let rows = markup.inline_keyboard;
            if rows.iter().any(|row| row.len() > Self::MAX_WIDTH)
                || rows.iter().map(Vec::len).sum::<usize>() > Self::MAX_BUTTONS
            {
                return Err(Error::Utility("invalid inline keyboard shape".to_owned()));
            }
            Ok(Self { rows })
        }

        #[allow(clippy::should_implement_trait)]
        pub fn add(mut self, button: InlineKeyboardButton) -> Result<Self> {
            if self.buttons().count() >= Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "inline keyboard has more than 100 buttons".to_owned(),
                ));
            }
            if self
                .rows
                .last()
                .is_none_or(|row| row.len() == Self::MAX_WIDTH)
            {
                self.rows.push(Vec::new());
            }
            self.rows.last_mut().unwrap().push(button);
            Ok(self)
        }

        pub fn add_many(
            mut self,
            buttons: impl IntoIterator<Item = InlineKeyboardButton>,
        ) -> Result<Self> {
            let buttons: Vec<_> = buttons.into_iter().collect();
            if self.buttons().count() + buttons.len() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "inline keyboard has more than 100 buttons".to_owned(),
                ));
            }
            for button in buttons {
                self = self.add(button)?;
            }
            Ok(self)
        }

        pub fn callback(self, text: impl Into<String>, data: impl Into<String>) -> Result<Self> {
            let mut button = InlineKeyboardButton::new(text);
            button.callback_data = Some(data.into());
            self.add(button)
        }

        /// Adds a callback button from a typed callback-data protocol.
        pub fn callback_data(
            self,
            text: impl Into<String>,
            data: &impl PackCallbackData,
        ) -> Result<Self> {
            self.callback(text, data.pack_callback_data()?)
        }

        pub fn url(self, text: impl Into<String>, url: impl Into<String>) -> Result<Self> {
            let mut button = InlineKeyboardButton::new(text);
            button.url = Some(url.into());
            self.add(button)
        }

        pub fn row(self, buttons: impl IntoIterator<Item = InlineKeyboardButton>) -> Result<Self> {
            self.row_with_width(buttons, Self::MAX_WIDTH)
        }

        pub fn row_with_width(
            mut self,
            buttons: impl IntoIterator<Item = InlineKeyboardButton>,
            width: usize,
        ) -> Result<Self> {
            let buttons: Vec<_> = buttons.into_iter().collect();
            if width == 0 || width > Self::MAX_WIDTH {
                return Err(Error::Utility(
                    "inline row width must be between 1 and 8".to_owned(),
                ));
            }
            if self.buttons().count() + buttons.len() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "inline keyboard has more than 100 buttons".to_owned(),
                ));
            }
            self.rows.extend(buttons.chunks(width).map(<[_]>::to_vec));
            Ok(self)
        }

        pub fn adjust(mut self, sizes: &[usize], repeat: bool) -> Result<Self> {
            let buttons: Vec<_> = self.buttons().cloned().collect();
            self.rows = adjusted(&buttons, sizes, repeat, Self::MAX_WIDTH)?;
            Ok(self)
        }

        pub fn attach(mut self, other: Self) -> Result<Self> {
            if self.buttons().count() + other.buttons().count() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "inline keyboard has more than 100 buttons".to_owned(),
                ));
            }
            self.rows.extend(other.rows);
            Ok(self)
        }

        pub fn buttons(&self) -> impl Iterator<Item = &InlineKeyboardButton> {
            self.rows.iter().flatten()
        }

        pub fn export(&self) -> Vec<Vec<InlineKeyboardButton>> {
            self.rows.clone()
        }

        pub fn build(self) -> InlineKeyboardMarkup {
            InlineKeyboardMarkup::new(self.rows)
        }
    }

    #[derive(Debug, Clone, Default)]
    pub struct ReplyKeyboardBuilder {
        rows: Vec<Vec<KeyboardButton>>,
    }

    impl ReplyKeyboardBuilder {
        pub const MAX_WIDTH: usize = 10;
        pub const MAX_BUTTONS: usize = 300;

        pub fn new() -> Self {
            Self::default()
        }

        pub fn from_markup(markup: ReplyKeyboardMarkup) -> Result<Self> {
            let rows = markup.keyboard;
            if rows.iter().any(|row| row.len() > Self::MAX_WIDTH)
                || rows.iter().map(Vec::len).sum::<usize>() > Self::MAX_BUTTONS
            {
                return Err(Error::Utility("invalid reply keyboard shape".to_owned()));
            }
            Ok(Self { rows })
        }

        #[allow(clippy::should_implement_trait)]
        pub fn add(mut self, button: KeyboardButton) -> Result<Self> {
            if self.buttons().count() >= Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "reply keyboard has more than 300 buttons".to_owned(),
                ));
            }
            if self
                .rows
                .last()
                .is_none_or(|row| row.len() == Self::MAX_WIDTH)
            {
                self.rows.push(Vec::new());
            }
            self.rows.last_mut().unwrap().push(button);
            Ok(self)
        }

        pub fn add_many(
            mut self,
            buttons: impl IntoIterator<Item = KeyboardButton>,
        ) -> Result<Self> {
            let buttons: Vec<_> = buttons.into_iter().collect();
            if self.buttons().count() + buttons.len() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "reply keyboard has more than 300 buttons".to_owned(),
                ));
            }
            for button in buttons {
                self = self.add(button)?;
            }
            Ok(self)
        }

        pub fn text(self, text: impl Into<String>) -> Result<Self> {
            self.add(KeyboardButton::new(text))
        }

        pub fn row(self, buttons: impl IntoIterator<Item = KeyboardButton>) -> Result<Self> {
            self.row_with_width(buttons, Self::MAX_WIDTH)
        }

        pub fn row_with_width(
            mut self,
            buttons: impl IntoIterator<Item = KeyboardButton>,
            width: usize,
        ) -> Result<Self> {
            let buttons: Vec<_> = buttons.into_iter().collect();
            if width == 0 || width > Self::MAX_WIDTH {
                return Err(Error::Utility(
                    "reply row width must be between 1 and 10".to_owned(),
                ));
            }
            if self.buttons().count() + buttons.len() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "reply keyboard has more than 300 buttons".to_owned(),
                ));
            }
            self.rows.extend(buttons.chunks(width).map(<[_]>::to_vec));
            Ok(self)
        }

        pub fn adjust(mut self, sizes: &[usize], repeat: bool) -> Result<Self> {
            let buttons: Vec<_> = self.buttons().cloned().collect();
            self.rows = adjusted(&buttons, sizes, repeat, Self::MAX_WIDTH)?;
            Ok(self)
        }

        pub fn buttons(&self) -> impl Iterator<Item = &KeyboardButton> {
            self.rows.iter().flatten()
        }

        pub fn export(&self) -> Vec<Vec<KeyboardButton>> {
            self.rows.clone()
        }

        pub fn attach(mut self, other: Self) -> Result<Self> {
            if self.buttons().count() + other.buttons().count() > Self::MAX_BUTTONS {
                return Err(Error::Utility(
                    "reply keyboard has more than 300 buttons".to_owned(),
                ));
            }
            self.rows.extend(other.rows);
            Ok(self)
        }

        pub fn build(self) -> ReplyKeyboardMarkup {
            ReplyKeyboardMarkup::new(self.rows)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    use super::backoff::{Backoff, BackoffConfig};
    use super::callback_data::CallbackData;
    use super::chat_action::{ChatActionConfig, ChatActionMiddleware, ChatActionSender};
    use super::deep_linking::{
        create_start_link, create_startapp_link_with_encoder, decode_payload,
    };
    use super::formatting::{Text, bold, html_bold, italic, markdown_v2_bold, text_link};
    use super::keyboard::{InlineKeyboardBuilder, ReplyKeyboardBuilder};
    use super::link::{
        AIOGRAM_BRANCH, ChannelBotLinkOptions, create_channel_bot_link,
        create_channel_bot_link_with_options, create_telegram_link, create_tg_link, docs_url,
    };
    use super::media_group::MediaGroupBuilder;
    use super::token;
    use super::web_app;

    crate::callback_data! {
        struct AdminAction("admin") {
            user_id: i64,
            action: String,
        }
    }

    crate::callback_data! {
        struct TogglePage("toggle") {
            enabled: bool,
            page: Option<u32>,
        }
    }

    crate::callback_data! {
        struct CustomSeparatorAction("custom", separator = "::") {
            user_id: i64,
            action: String,
        }
    }

    #[test]
    fn callback_data_roundtrip_and_limit() {
        let protocol = CallbackData::new("admin")
            .unwrap()
            .push(42)
            .unwrap()
            .push("ban")
            .unwrap();
        let packed = protocol.pack().unwrap();
        assert_eq!(packed, "admin:42:ban");
        assert_eq!(protocol.unpack(&packed).unwrap(), vec!["42", "ban"]);

        let typed = AdminAction::new(42, "ban".to_owned());
        assert_eq!(typed.pack().unwrap(), "admin:42:ban");
        assert_eq!(AdminAction::unpack("admin:42:ban").unwrap(), typed);
        assert!(AdminAction::unpack("admin:not-a-number:ban").is_err());

        let toggle = TogglePage::new(true, None);
        assert_eq!(toggle.pack().unwrap(), "toggle:1:");
        assert_eq!(TogglePage::unpack("toggle:1:").unwrap(), toggle);
        assert_eq!(TogglePage::unpack("toggle:0:12").unwrap().page, Some(12));
        assert!(TogglePage::unpack("toggle:true:").is_err());

        let runtime = CallbackData::new("toggle")
            .unwrap()
            .push_value(true)
            .unwrap()
            .push_value(None::<u32>)
            .unwrap();
        assert_eq!(runtime.pack().unwrap(), "toggle:1:");

        let runtime = CallbackData::with_separator("runtime", "::")
            .unwrap()
            .push(42)
            .unwrap()
            .push("open")
            .unwrap();
        assert_eq!(runtime.separator(), "::");
        assert_eq!(runtime.pack().unwrap(), "runtime::42::open");
        assert_eq!(runtime.unpack("runtime::42::open").unwrap(), ["42", "open"]);
        assert!(CallbackData::with_separator("runtime", "").is_err());

        let legacy_char_separator = CallbackData::with_separator("legacy", '|').unwrap();
        assert_eq!(legacy_char_separator.separator(), "|");

        let typed = CustomSeparatorAction::new(42, "open".to_owned());
        assert_eq!(typed.pack().unwrap(), "custom::42::open");
        assert_eq!(
            CustomSeparatorAction::unpack("custom::42::open").unwrap(),
            typed
        );
    }

    #[test]
    fn deep_link_encodes_arbitrary_payload() {
        let link = create_start_link("@sample_bot", "hello world", true).unwrap();
        let payload = link.split('=').nth(1).unwrap();
        assert_eq!(decode_payload(payload).unwrap(), b"hello world");

        let reverse = |value: &[u8]| value.iter().rev().copied().collect::<Vec<_>>();
        let encoded = super::payload::encode_payload_with("secret", reverse);
        assert_eq!(
            create_startapp_link_with_encoder("sample_bot", "secret", Some("app"), reverse)
                .unwrap(),
            format!("https://t.me/sample_bot/app?startapp={encoded}")
        );
        assert!(
            create_startapp_link_with_encoder("sample_bot", &"x".repeat(49), None, reverse)
                .is_err()
        );
        assert_eq!(
            super::deep_linking::create_startgroup_link("sample_bot", "group", false).unwrap(),
            "https://t.me/sample_bot?startgroup=group"
        );
        assert_eq!(
            super::deep_linking::create_startapp_link("sample_bot", "payload", false, Some("shop"))
                .unwrap(),
            "https://t.me/sample_bot/shop?startapp=payload"
        );
    }

    #[test]
    fn payload_and_serialization_helpers_match_aiogram_workflows() {
        let encoded = super::payload::encode_payload_with("secret", |bytes| {
            bytes.iter().map(|byte| byte ^ 0x55).collect()
        });
        assert_eq!(
            super::payload::decode_payload_with(&encoded, |bytes| {
                bytes.iter().map(|byte| byte ^ 0x55).collect()
            })
            .unwrap(),
            "secret"
        );

        let method = crate::methods::SendPhoto::new(
            42_i64,
            crate::types::InputFile::named_bytes("photo", "photo.jpg", b"image".to_vec()),
        );
        let serialized = super::serialization::deserialize_method(&method, true).unwrap();
        assert_eq!(serialized.data["method"], "sendPhoto");
        assert_eq!(serialized.data["photo"], "attach://photo");
        assert_eq!(serialized.files.len(), 1);
        assert_eq!(serialized.files[0].file_name, "photo.jpg");
    }

    #[test]
    fn builds_aiogram_compatible_telegram_links() {
        assert_eq!(create_tg_link("user", [("id", 42)]), "tg://user?id=42");
        assert_eq!(
            create_telegram_link(&["sample_bot"]),
            "https://t.me/sample_bot"
        );
        assert_eq!(
            docs_url(&["test.html"], Some("filtering-events")),
            format!("https://docs.aiogram.dev/en/{AIOGRAM_BRANCH}/test.html#filtering-events")
        );
        assert_eq!(
            create_channel_bot_link("sample_bot"),
            "https://t.me/sample_bot"
        );

        let options = ChannelBotLinkOptions {
            change_info: true,
            delete_messages: true,
            ..ChannelBotLinkOptions::default().parameter("parameter in group")
        };
        assert_eq!(
            create_channel_bot_link_with_options("sample_bot", &options),
            "https://t.me/sample_bot?startgroup=parameter+in+group&admin=change_info%2Bdelete_messages"
        );
    }

    #[test]
    fn formatting_escapes_user_content() {
        assert_eq!(html_bold("a < b"), "<b>a &lt; b</b>");
        assert_eq!(markdown_v2_bold("a.b"), "*a\\.b*");
        assert_eq!(
            super::formatting::html_quote("say \"hi\" < now"),
            "say \"hi\" &lt; now"
        );
        assert_eq!(super::formatting::markdown_v2_italic("a.b"), "_\ra\\.b_\r");
        assert_eq!(
            super::formatting::html_underline("a < b"),
            "<u>a &lt; b</u>"
        );
        assert_eq!(
            super::formatting::markdown_v2_strikethrough("a.b"),
            "~a\\.b~"
        );
        assert_eq!(
            super::formatting::hide_link("https://example.com/image.jpg"),
            "<a href=\"https://example.com/image.jpg\">&#8203;</a>"
        );
        assert_eq!(
            super::formatting::html_custom_emoji("🙂", "emoji-id"),
            "<tg-emoji emoji-id=\"emoji-id\">🙂</tg-emoji>"
        );
        assert_eq!(
            super::formatting::markdown_v2_custom_emoji("🙂", "emoji-id"),
            "![🙂](tg://emoji?emoji_id=emoji-id)"
        );
        assert_eq!(
            super::formatting::html_date_time("now", 42, Some("d MMM")),
            "<tg-time unix=\"42\" format=\"d MMM\">now</tg-time>"
        );
        assert_eq!(
            super::formatting::markdown_v2_expandable_blockquote("one\ntwo"),
            ">one\n>two||"
        );
    }

    #[test]
    fn entity_formatting_uses_utf16_offsets_and_nested_entities() {
        assert_eq!(super::formatting::sizeof("🙂A"), 3);
        let formatted = Text::plain("🙂 ")
            .then(bold(Text::plain("A").then(italic("Б"))))
            .then(" ")
            .then(text_link("docs", "https://example.com"));
        let rendered = formatted.render();
        assert_eq!(rendered.text, "🙂 AБ docs");
        assert_eq!(rendered.entities.len(), 3);
        assert_eq!(rendered.entities[0].kind, "bold");
        assert_eq!(rendered.entities[0].offset, 3);
        assert_eq!(rendered.entities[0].length, 2);
        assert_eq!(rendered.entities[1].kind, "italic");
        assert_eq!(rendered.entities[1].offset, 4);
        assert_eq!(rendered.entities[2].kind, "text_link");
        assert_eq!(
            rendered.entities[2].url.as_deref(),
            Some("https://example.com")
        );

        let method = Text::plain("hello")
            .then(bold(" world"))
            .into_send_message(42_i64);
        let json = serde_json::to_value(method).unwrap();
        assert_eq!(json["text"], "hello world");
        assert_eq!(json["entities"][0]["offset"], 5);
        assert!(json["parse_mode"].is_null());

        assert_eq!(
            super::formatting::as_marked_section(
                bold("Tasks"),
                [Text::plain("one"), Text::plain("two")]
            )
            .render()
            .text,
            "Tasks\n- one\n- two"
        );
        assert_eq!(
            super::formatting::as_line_with(["one", "two"], "!", ", ")
                .render()
                .text,
            "one, two!"
        );
    }

    #[test]
    fn formatted_text_slices_utf16_without_losing_nested_entities() {
        let formatted = Text::plain("🙂 ")
            .then(bold(Text::plain("World").then(italic("!"))))
            .then(" tail");

        let sliced = formatted.slice_utf16(3..7).unwrap().render();
        assert_eq!(sliced.text, "Worl");
        assert_eq!(sliced.entities.len(), 1);
        assert_eq!(sliced.entities[0].kind, "bold");
        assert_eq!(sliced.entities[0].offset, 0);
        assert_eq!(sliced.entities[0].length, 4);

        let nested = formatted.slice_utf16(7..9).unwrap().render();
        assert_eq!(nested.text, "d!");
        assert_eq!(
            nested
                .entities
                .iter()
                .map(|entity| (entity.kind.as_str(), entity.offset, entity.length))
                .collect::<Vec<_>>(),
            vec![("bold", 0, 2), ("italic", 1, 1)]
        );

        assert!(formatted.slice_utf16(1..2).is_err());
        let utf16_len = formatted.render().text.encode_utf16().count();
        assert!(
            formatted
                .slice_utf16(utf16_len..utf16_len.saturating_sub(1))
                .is_err()
        );
        assert!(formatted.slice_utf16(0..100).is_err());
    }

    #[test]
    fn formatted_text_exports_fields_and_replaces_root() {
        let original = bold(Text::plain("old ").then(italic("value")));
        let replaced = original.replace([Text::plain("new ").then(italic("value"))]);
        let rendered = replaced.render();
        assert_eq!(rendered.text, "new value");
        assert_eq!(
            rendered
                .entities
                .iter()
                .map(|entity| (entity.kind.as_str(), entity.offset, entity.length))
                .collect::<Vec<_>>(),
            [("bold", 0, 9), ("italic", 4, 5)]
        );

        let kwargs = replaced.as_kwargs().unwrap();
        assert_eq!(kwargs["text"], "new value");
        assert_eq!(kwargs["entities"][0]["type"], "bold");
        assert!(kwargs["parse_mode"].is_null());
        let caption = replaced.as_caption_kwargs().unwrap();
        assert_eq!(caption["caption"], "new value");
        assert!(caption["parse_mode"].is_null());
        let question = replaced.as_poll_question_kwargs().unwrap();
        assert!(question["question_parse_mode"].is_null());
        let explanation = replaced.as_poll_explanation_kwargs().unwrap();
        assert!(explanation["explanation_parse_mode"].is_null());
        let gift = replaced.as_gift_text_kwargs().unwrap();
        assert!(gift["text_parse_mode"].is_null());
        let custom = replaced
            .as_fields("body", "body_entities", "body_parse_mode", false)
            .unwrap();
        assert_eq!(custom["body"], "new value");
        assert!(custom.get("body_parse_mode").is_none());
        assert!(replaced.as_pretty_string(true).contains("Text"));

        assert_eq!(Text::plain("old").replace(["new"]).render().text, "new");
    }

    #[test]
    fn formatted_text_suppresses_default_parse_mode_for_text_and_captions() {
        let bot = crate::Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .defaults(crate::DefaultBotProperties::default().parse_mode("HTML"))
            .build()
            .unwrap();

        let message = Text::plain("Hello, ")
            .then(bold("world"))
            .into_send_message(42_i64);
        let request = bot.prepare_request(&message).unwrap();
        assert_eq!(request.payload["text"], "Hello, world");
        assert_eq!(request.payload["entities"][0]["type"], "bold");
        assert!(request.payload.get("parse_mode").is_none());

        let photo = Text::plain("Photo ")
            .then(italic("caption"))
            .apply_caption(crate::methods::SendPhoto::new(42_i64, "telegram-file-id"));
        let request = bot.prepare_request(&photo).unwrap();
        assert_eq!(request.payload["caption"], "Photo caption");
        assert_eq!(request.payload["caption_entities"][0]["type"], "italic");
        assert!(request.payload.get("parse_mode").is_none());

        let poll = Text::plain("Question ")
            .then(bold("one"))
            .apply_poll_question(crate::methods::SendPoll::new(42_i64, "old", Vec::new()));
        let poll = Text::plain("Because ")
            .then(italic("two"))
            .apply_poll_explanation(poll);
        let request = bot.prepare_request(&poll).unwrap();
        assert_eq!(request.payload["question"], "Question one");
        assert_eq!(request.payload["question_entities"][0]["type"], "bold");
        assert!(request.payload.get("question_parse_mode").is_none());
        assert_eq!(request.payload["explanation"], "Because two");
        assert_eq!(request.payload["explanation_entities"][0]["type"], "italic");
        assert!(request.payload.get("explanation_parse_mode").is_none());
        assert_eq!(request.payload["description_parse_mode"], "HTML");

        let gift = Text::plain("Gift ")
            .then(bold("note"))
            .apply_gift_text(crate::methods::SendGift::new("gift-id"));
        let payload = serde_json::to_value(gift).unwrap();
        assert_eq!(payload["text"], "Gift note");
        assert_eq!(payload["text_entities"][0]["type"], "bold");
        assert!(payload.get("text_parse_mode").is_none());

        let edit = Text::plain("Edited ")
            .then(italic("text"))
            .apply_text(crate::methods::EditMessageText::new());
        let request = bot.prepare_request(&edit).unwrap();
        assert_eq!(request.payload["text"], "Edited text");
        assert_eq!(request.payload["entities"][0]["type"], "italic");
        assert!(request.payload.get("parse_mode").is_none());
    }

    #[test]
    fn entity_formatting_roundtrips_to_html_and_markdown() {
        let source = "🙂 test1 test2!";
        let entities = vec![
            crate::types::MessageEntity::new("bold", 3, 11),
            crate::types::MessageEntity::new("underline", 9, 5),
        ];
        let text = Text::from_entities(source, &entities).unwrap();
        assert_eq!(text.as_html(), "🙂 <b>test1 <u>test2</u></b>!");
        assert_eq!(text.as_markdown(), "🙂 *test1 __\rtest2__\r*\\!");
        let rendered = text.render();
        assert_eq!(rendered.text, source);
        assert_eq!(rendered.entities.len(), 2);
        assert_eq!(rendered.entities[0].offset, 3);

        let invalid = crate::types::MessageEntity::new("bold", 1, 1);
        assert!(Text::from_entities("🙂", &[invalid]).is_err());
    }

    #[test]
    fn keyboard_builder_adjusts_rows() {
        let markup = InlineKeyboardBuilder::new()
            .callback("one", "1")
            .unwrap()
            .callback("two", "2")
            .unwrap()
            .callback("three", "3")
            .unwrap()
            .adjust(&[2, 1], false)
            .unwrap()
            .build();
        assert_eq!(
            markup
                .inline_keyboard
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        let rebuilt = InlineKeyboardBuilder::from_markup(markup.clone()).unwrap();
        assert_eq!(rebuilt.export(), markup.inline_keyboard);

        let action = AdminAction::new(42, "open".to_owned());
        let typed_markup = InlineKeyboardBuilder::new()
            .callback_data("Open", &action)
            .unwrap()
            .build();
        assert_eq!(
            typed_markup.inline_keyboard[0][0].callback_data.as_deref(),
            Some("admin:42:open")
        );

        let runtime = CallbackData::new("runtime").unwrap().push("close").unwrap();
        let runtime_markup = InlineKeyboardBuilder::new()
            .callback_data("Close", &runtime)
            .unwrap()
            .build();
        assert_eq!(
            runtime_markup.inline_keyboard[0][0]
                .callback_data
                .as_deref(),
            Some("runtime:close")
        );

        let inline_row = InlineKeyboardBuilder::new()
            .row((0..9).map(|index| crate::types::InlineKeyboardButton::new(index.to_string())))
            .unwrap()
            .build();
        assert_eq!(
            inline_row
                .inline_keyboard
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            [8, 1]
        );
        let inline_width = InlineKeyboardBuilder::new()
            .row_with_width(
                (0..9).map(|index| crate::types::InlineKeyboardButton::new(index.to_string())),
                3,
            )
            .unwrap()
            .build();
        assert_eq!(
            inline_width
                .inline_keyboard
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            [3, 3, 3]
        );
        let inline_many = InlineKeyboardBuilder::new()
            .add_many(
                (0..9).map(|index| crate::types::InlineKeyboardButton::new(index.to_string())),
            )
            .unwrap()
            .build();
        assert_eq!(
            inline_many
                .inline_keyboard
                .iter()
                .map(Vec::len)
                .collect::<Vec<_>>(),
            [8, 1]
        );

        let reply = ReplyKeyboardBuilder::new()
            .text("one")
            .unwrap()
            .attach(ReplyKeyboardBuilder::new().text("two").unwrap())
            .unwrap()
            .adjust(&[2], false)
            .unwrap()
            .build();
        assert_eq!(reply.keyboard[0].len(), 2);
        assert_eq!(
            ReplyKeyboardBuilder::from_markup(reply.clone())
                .unwrap()
                .export(),
            reply.keyboard
        );

        let reply_row = ReplyKeyboardBuilder::new()
            .row((0..11).map(|index| crate::types::KeyboardButton::new(index.to_string())))
            .unwrap()
            .build();
        assert_eq!(
            reply_row.keyboard.iter().map(Vec::len).collect::<Vec<_>>(),
            [10, 1]
        );
        let reply_many = ReplyKeyboardBuilder::new()
            .add_many((0..11).map(|index| crate::types::KeyboardButton::new(index.to_string())))
            .unwrap()
            .build();
        assert_eq!(
            reply_many.keyboard.iter().map(Vec::len).collect::<Vec<_>>(),
            [10, 1]
        );
    }

    #[test]
    fn backoff_grows_and_resets_like_polling_schedule() {
        let config = BackoffConfig::new(
            Duration::from_secs(1),
            Duration::from_secs(5),
            2.0,
            Duration::ZERO,
        )
        .unwrap();
        let mut backoff = Backoff::new(config);
        assert_eq!(backoff.min_delay(), Duration::from_secs(1));
        assert_eq!(backoff.max_delay(), Duration::from_secs(5));
        assert_eq!(backoff.factor(), 2.0);
        assert_eq!(backoff.jitter(), Duration::ZERO);
        assert_eq!(backoff.next(), Some(Duration::from_secs(1)));
        assert_eq!(backoff.advance(), Duration::from_secs(2));
        assert_eq!(backoff.advance(), Duration::from_secs(4));
        assert_eq!(backoff.advance(), Duration::from_secs(5));
        assert_eq!(backoff.counter(), 4);
        assert_eq!(
            backoff.to_string(),
            "Backoff(tryings=4, current_delay=5, next_delay=5)"
        );
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
        assert_eq!(backoff.counter(), 0);
    }

    #[test]
    fn validates_token_and_extracts_bot_id() {
        assert_eq!(
            token::extract_bot_id("123456:abcdefghijklmnopqrstuvwxyz").unwrap(),
            123456
        );
        assert!(token::validate("bad token").is_err());
    }

    #[test]
    fn validates_and_parses_web_app_init_data() {
        let token = "123456:abcdefghijklmnopqrstuvwxyz";
        let mut fields = BTreeMap::from([
            ("auth_date", "1700000000"),
            ("query_id", "AAEAAAE"),
            (
                "user",
                r#"{"id":42,"first_name":"Ada","language_code":"en"}"#,
            ),
        ]);
        let check_string = fields
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut key = Hmac::<Sha256>::new_from_slice(b"WebAppData").unwrap();
        key.update(token.as_bytes());
        let mut mac = Hmac::<Sha256>::new_from_slice(&key.finalize().into_bytes()).unwrap();
        mac.update(check_string.as_bytes());
        let hash = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fields.insert("hash", &hash);
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .finish();

        assert!(web_app::check_signature(token, &encoded));
        let parsed = web_app::safe_parse_init_data(token, &encoded).unwrap();
        assert_eq!(parsed.auth_date, 1_700_000_000);
        assert_eq!(parsed.user.unwrap().first_name, "Ada");
        assert!(!web_app::check_signature("another-token", &encoded));
    }

    #[test]
    fn validates_third_party_web_app_ed25519_signature() {
        let public_key = [
            0x41, 0x12, 0x76, 0x50, 0x21, 0x34, 0x1e, 0x54, 0x15, 0xe7, 0x72, 0xcd, 0x65, 0x90,
            0x3f, 0x6b, 0x94, 0xe3, 0xea, 0x1c, 0x2a, 0xb6, 0x69, 0xe6, 0xd3, 0xe1, 0x8e, 0xe2,
            0xdb, 0x00, 0xda, 0x61,
        ];
        let init_data = "auth_date=1650385342&user=%7B%22id%22%3A42%2C%22first_name%22%3A%22Test%22%7D&query_id=test&hash=123&signature=JQ0JR2tjC65yq_jNZV0wuJVX6J-SWPMV0mprUXG34g-NvxL4RcF1Rz5n4VVo00VRghEUBf5t___uoeb1-jU_Cw";

        assert!(web_app::check_signature_with_public_key(
            42,
            init_data,
            &public_key
        ));
        let parsed =
            web_app::safe_parse_init_data_with_public_key(42, init_data, &public_key).unwrap();
        assert_eq!(parsed.query_id.as_deref(), Some("test"));
        assert_eq!(parsed.user.unwrap().id, 42);
        assert!(!web_app::check_signature_with_public_key(
            43,
            init_data,
            &public_key
        ));
    }

    #[test]
    fn media_group_applies_caption_and_keeps_nested_uploads() {
        let media = MediaGroupBuilder::new()
            .caption("album")
            .add_photo(crate::types::InputFile::bytes(
                "first.jpg",
                b"first".to_vec(),
            ))
            .unwrap()
            .add_video("telegram-video-id")
            .unwrap()
            .build();
        assert_eq!(media.len(), 2);
        let json = serde_json::to_value(&media).unwrap();
        assert_eq!(json[0]["type"], "photo");
        assert_eq!(json[0]["caption"], "album");
        assert_eq!(json[0]["media"], "attach://first.jpg");
        assert!(json[1].get("caption").is_none());
    }

    #[test]
    fn chat_action_sender_exposes_every_aiogram_factory() {
        let bot = crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
        let actions = [
            ChatActionSender::typing(bot.clone(), 42_i64),
            ChatActionSender::upload_photo(bot.clone(), 42_i64),
            ChatActionSender::record_video(bot.clone(), 42_i64),
            ChatActionSender::upload_video(bot.clone(), 42_i64),
            ChatActionSender::record_voice(bot.clone(), 42_i64),
            ChatActionSender::upload_voice(bot.clone(), 42_i64),
            ChatActionSender::upload_document(bot.clone(), 42_i64),
            ChatActionSender::choose_sticker(bot.clone(), 42_i64),
            ChatActionSender::find_location(bot.clone(), 42_i64),
            ChatActionSender::record_video_note(bot.clone(), 42_i64),
            ChatActionSender::upload_video_note(bot.clone(), 42_i64),
        ];
        assert_eq!(
            actions.map(|sender| sender.action().to_owned()),
            [
                "typing",
                "upload_photo",
                "record_video",
                "upload_video",
                "record_voice",
                "upload_voice",
                "upload_document",
                "choose_sticker",
                "find_location",
                "record_video_note",
                "upload_video_note",
            ]
        );

        let sender = ChatActionSender::upload_voice(bot, 7_i64)
            .message_thread_id(11)
            .interval(Duration::from_secs(4))
            .initial_sleep(Duration::from_secs(2));
        assert_eq!(sender.chat_id(), &crate::types::ChatId::Id(7));
        assert_eq!(sender.message_thread(), Some(11));
        assert_eq!(sender.repeat_interval(), Duration::from_secs(4));
        assert_eq!(sender.initial_delay(), Duration::from_secs(2));
        assert_eq!(sender.bot().id(), 123456);
    }

    #[tokio::test]
    async fn chat_action_sender_starts_and_stops_without_leaking_task() {
        let bot = crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
        let mut sender =
            ChatActionSender::typing(bot, 42_i64).initial_sleep(Duration::from_secs(60));
        sender.start().unwrap();
        assert!(sender.is_running());
        assert!(sender.start().is_err());
        sender.stop().await.unwrap();
        assert!(!sender.is_running());
    }

    #[tokio::test]
    async fn chat_action_middleware_uses_typed_handler_flag() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let called = Arc::new(AtomicBool::new(false));
        let captured = called.clone();
        let mut router = crate::Router::new();
        router.middleware(ChatActionMiddleware::new());
        router.message_with_flags(
            crate::filters::any(),
            crate::HandlerFlags::new().with(
                "chat_action",
                ChatActionConfig::new()
                    .action("upload_photo")
                    .initial_sleep(Duration::from_secs(60)),
            ),
            move |_| {
                let captured = captured.clone();
                async move {
                    captured.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let mut dispatcher = crate::Dispatcher::new();
        dispatcher.include_router(router);
        let update = serde_json::from_value(serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 4, "type": "private"},
                "text": "hello"
            }
        }))
        .unwrap();

        assert!(
            dispatcher
                .feed_update(
                    crate::Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
                    update,
                )
                .await
                .unwrap()
        );
        assert!(called.load(Ordering::SeqCst));
    }
}
