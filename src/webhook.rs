//! Framework-neutral webhook handling and optional Axum integration.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use subtle::ConstantTimeEq;

use crate::bot::Bot;
use crate::client::BotRequest;
use crate::dispatcher::Dispatcher;
use crate::error::{Error, Result};
use crate::types::Update;

pub const SECRET_TOKEN_HEADER: &str = "x-telegram-bot-api-secret-token";
pub const DEFAULT_TELEGRAM_NETWORKS: [&str; 2] = ["149.154.160.0/20", "91.108.4.0/22"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ipv4Range {
    first: u32,
    last: u32,
}

/// IPv4 allow-list equivalent to aiogram's webhook `IPFilter`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IpFilter {
    ranges: Vec<Ipv4Range>,
}

impl IpFilter {
    pub fn new(ips: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Self> {
        let mut filter = Self::default();
        filter.allow(ips)?;
        Ok(filter)
    }

    pub fn telegram_default() -> Self {
        Self::new(DEFAULT_TELEGRAM_NETWORKS)
            .expect("Telegram's built-in webhook networks are valid CIDR ranges")
    }

    pub fn allow(&mut self, ips: impl IntoIterator<Item = impl AsRef<str>>) -> Result<&mut Self> {
        for ip in ips {
            self.allow_ip(ip.as_ref())?;
        }
        Ok(self)
    }

    pub fn allow_ip(&mut self, value: &str) -> Result<&mut Self> {
        let range = if let Some((address, prefix)) = value.split_once('/') {
            let address = address.parse::<Ipv4Addr>().map_err(|error| {
                Error::InvalidPayload(format!("invalid IPv4 address {address:?}: {error}"))
            })?;
            let prefix = prefix.parse::<u8>().map_err(|error| {
                Error::InvalidPayload(format!("invalid IPv4 prefix {prefix:?}: {error}"))
            })?;
            if prefix > 32 {
                return Err(Error::InvalidPayload(format!(
                    "IPv4 prefix must be between 0 and 32, got {prefix}"
                )));
            }
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            let network = u32::from(address) & mask;
            let broadcast = network | !mask;
            if prefix <= 30 {
                Ipv4Range {
                    first: network.saturating_add(1),
                    last: broadcast.saturating_sub(1),
                }
            } else {
                Ipv4Range {
                    first: network,
                    last: broadcast,
                }
            }
        } else {
            let address = value.parse::<Ipv4Addr>().map_err(|error| {
                Error::InvalidPayload(format!("invalid IPv4 address {value:?}: {error}"))
            })?;
            let address = u32::from(address);
            Ipv4Range {
                first: address,
                last: address,
            }
        };
        if !self.ranges.contains(&range) {
            self.ranges.push(range);
        }
        Ok(self)
    }

    pub fn check(&self, value: &str) -> bool {
        value.parse::<Ipv4Addr>().is_ok_and(|address| {
            let address = u32::from(address);
            self.ranges
                .iter()
                .any(|range| range.first <= address && address <= range.last)
        })
    }

    /// Uses the left-most proxy address when `X-Forwarded-For` is present,
    /// matching aiogram's aiohttp middleware behavior.
    pub fn check_client(
        &self,
        forwarded_for: Option<&str>,
        peer_address: Option<Ipv4Addr>,
    ) -> (String, bool) {
        let address = forwarded_for
            .and_then(|value| value.split(',').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| peer_address.map(|value| value.to_string()))
            .unwrap_or_default();
        let accepted = self.check(&address);
        (address, accepted)
    }
}

type BotFactory = Arc<dyn Fn(&str) -> Result<Bot> + Send + Sync>;

/// Lazily resolves and caches bots by URL token, corresponding to aiogram's
/// `TokenBasedRequestHandler`. Tokens in URLs may be logged by proxies, so a
/// secret-token single-bot route remains the recommended deployment model.
#[derive(Clone)]
pub struct TokenBotRegistry {
    bots: Arc<dashmap::DashMap<String, Bot>>,
    factory: BotFactory,
}

impl Default for TokenBotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenBotRegistry {
    pub fn new() -> Self {
        Self::with_factory(|token| Bot::new(token.to_owned()))
    }

    pub fn with_factory(factory: impl Fn(&str) -> Result<Bot> + Send + Sync + 'static) -> Self {
        Self {
            bots: Arc::new(dashmap::DashMap::new()),
            factory: Arc::new(factory),
        }
    }

    pub fn resolve(&self, token: &str) -> Result<Bot> {
        if let Some(bot) = self.bots.get(token) {
            return Ok(bot.clone());
        }
        let bot = (self.factory)(token)?;
        Ok(self.bots.entry(token.to_owned()).or_insert(bot).clone())
    }

    pub fn len(&self) -> usize {
        self.bots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bots.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebhookResponse {
    Json(serde_json::Value),
    Multipart { content_type: String, body: Vec<u8> },
}

/// Verifies Telegram's webhook secret header in constant time.
pub fn verify_secret_token(provided: Option<&str>, expected: Option<&str>) -> bool {
    match (provided, expected) {
        (_, None) => true,
        (Some(provided), Some(expected)) if provided.len() == expected.len() => {
            provided.as_bytes().ct_eq(expected.as_bytes()).into()
        }
        _ => false,
    }
}

/// Deserializes and dispatches one Telegram webhook update.
pub async fn feed_json(dispatcher: &Dispatcher, bot: Bot, body: &[u8]) -> Result<bool> {
    let update: Update = serde_json::from_slice(body)?;
    dispatcher.feed_update(bot, update).await
}

/// Dispatches an update and returns an optional Bot API method to send as the
/// webhook HTTP response.
pub async fn feed_json_with_response(
    dispatcher: &Dispatcher,
    bot: Bot,
    body: &[u8],
) -> Result<(bool, Option<WebhookResponse>)> {
    let update: Update = serde_json::from_slice(body)?;
    let (handled, response) = dispatcher.feed_webhook_update(bot.clone(), update).await?;
    let response = match response {
        Some(response) => Some(prepare_response(&bot, response).await?),
        None => None,
    };
    Ok((handled, response))
}

/// Parses the request synchronously, then dispatches it on a Tokio task and
/// sends a returned webhook method through the Bot API in the background.
pub fn feed_json_in_background(
    dispatcher: Arc<Dispatcher>,
    bot: Bot,
    body: &[u8],
) -> Result<tokio::task::JoinHandle<Result<()>>> {
    let update: Update = serde_json::from_slice(body)?;
    Ok(tokio::spawn(async move {
        let (_, response) = dispatcher.feed_webhook_update(bot.clone(), update).await?;
        if let Some(response) = response {
            bot.send_request(response).await?;
        }
        Ok(())
    }))
}

async fn prepare_response(bot: &Bot, request: BotRequest) -> Result<WebhookResponse> {
    let mut payload = request.payload;
    let object = payload.as_object_mut().ok_or_else(|| {
        Error::InvalidPayload("webhook method payload must be an object".to_owned())
    })?;
    object.insert(
        "method".to_owned(),
        serde_json::Value::String(request.method_name),
    );
    if request.files.is_empty() {
        return Ok(WebhookResponse::Json(payload));
    }

    let boundary = format!(
        "aiogram-rust-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let mut body = Vec::new();
    for (name, value) in object {
        if value.is_null() {
            continue;
        }
        let value = match value {
            serde_json::Value::String(value) => value.clone(),
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.to_string(),
            _ => serde_json::to_string(value)?,
        };
        push_multipart_field(&mut body, &boundary, name, value.as_bytes())?;
    }
    for file in request.files {
        let name = safe_disposition_value(&file.attachment_name)?;
        let file_name = safe_disposition_value(&file.file_name)?;
        let content = bot.load_input_file(file.content).await?;
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok(WebhookResponse::Multipart {
        content_type: format!("multipart/form-data; boundary={boundary}"),
        body,
    })
}

fn push_multipart_field(
    body: &mut Vec<u8>,
    boundary: &str,
    name: &str,
    value: &[u8],
) -> Result<()> {
    let name = safe_disposition_value(name)?;
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(value);
    body.extend_from_slice(b"\r\n");
    Ok(())
}

fn safe_disposition_value(value: &str) -> Result<String> {
    if value
        .chars()
        .any(|character| matches!(character, '\r' | '\n'))
    {
        return Err(Error::InvalidPayload(
            "multipart names cannot contain line breaks".to_owned(),
        ));
    }
    Ok(value.replace('"', "%22"))
}

#[cfg(feature = "webhook-axum")]
pub mod axum_integration {
    use std::future::Future;
    use std::net::{IpAddr, SocketAddr};
    use std::sync::Arc;

    use axum::Extension;
    use axum::Router;
    use axum::body::Bytes;
    use axum::extract::{ConnectInfo, Path, State};
    use axum::http::header::CONTENT_TYPE;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use axum::response::{IntoResponse, Response};
    use axum::routing::post;

    use super::{
        IpFilter, SECRET_TOKEN_HEADER, TokenBotRegistry, WebhookResponse, feed_json_in_background,
        feed_json_with_response, verify_secret_token,
    };
    use crate::{Bot, Dispatcher};

    #[derive(Clone)]
    struct WebhookState {
        dispatcher: Arc<Dispatcher>,
        bot: Bot,
        secret_token: Option<Arc<str>>,
        handle_in_background: bool,
        ip_filter: Option<IpFilter>,
    }

    #[derive(Clone)]
    struct TokenWebhookState {
        dispatcher: Arc<Dispatcher>,
        bots: TokenBotRegistry,
        handle_in_background: bool,
    }

    #[derive(Debug, Clone, Default)]
    pub struct WebhookOptions {
        pub secret_token: Option<String>,
        pub handle_in_background: bool,
        pub ip_filter: Option<IpFilter>,
    }

    impl WebhookOptions {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn secret_token(mut self, value: impl Into<String>) -> Self {
            self.secret_token = Some(value.into());
            self
        }

        pub fn handle_in_background(mut self, value: bool) -> Self {
            self.handle_in_background = value;
            self
        }

        pub fn ip_filter(mut self, value: IpFilter) -> Self {
            self.ip_filter = Some(value);
            self
        }
    }

    /// Creates an Axum router that accepts Telegram webhook POST requests.
    pub fn router(
        path: &str,
        dispatcher: Arc<Dispatcher>,
        bot: Bot,
        secret_token: Option<String>,
    ) -> Router {
        router_with_options(
            path,
            dispatcher,
            bot,
            WebhookOptions {
                secret_token,
                ..WebhookOptions::default()
            },
        )
    }

    pub fn router_with_options(
        path: &str,
        dispatcher: Arc<Dispatcher>,
        bot: Bot,
        options: WebhookOptions,
    ) -> Router {
        let state = WebhookState {
            dispatcher,
            bot,
            secret_token: options.secret_token.map(Arc::from),
            handle_in_background: options.handle_in_background,
            ip_filter: options.ip_filter,
        };
        Router::new().route(path, post(handle)).with_state(state)
    }

    /// Creates a multi-bot route whose path must contain `{bot_token}`. As in
    /// upstream aiogram, this is supported for compatibility but discouraged
    /// because infrastructure access logs may expose bot tokens.
    pub fn token_router(
        path: &str,
        dispatcher: Arc<Dispatcher>,
        bots: TokenBotRegistry,
        handle_in_background: bool,
    ) -> crate::Result<Router> {
        if !path.contains("{bot_token}") {
            return Err(crate::Error::InvalidPayload(
                "token webhook path must contain `{bot_token}`".to_owned(),
            ));
        }
        let state = TokenWebhookState {
            dispatcher,
            bots,
            handle_in_background,
        };
        Ok(Router::new()
            .route(path, post(handle_token))
            .with_state(state))
    }

    /// Runs an Axum webhook router with dispatcher startup/shutdown hooks and
    /// graceful shutdown. This is the Rust counterpart of aiogram's
    /// `setup_application` lifecycle wiring.
    pub async fn serve<F>(
        listener: tokio::net::TcpListener,
        app: Router,
        dispatcher: Arc<Dispatcher>,
        bot: Bot,
        shutdown: F,
    ) -> crate::Result<()>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        dispatcher.emit_startup(bot.clone()).await?;
        let server_result = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
            .map_err(crate::Error::Io);
        let shutdown_result = dispatcher.emit_shutdown(bot).await;
        server_result.and(shutdown_result)
    }

    fn method_response(response: Option<WebhookResponse>) -> Response {
        match response {
            Some(WebhookResponse::Json(answer)) => {
                (StatusCode::OK, axum::Json(answer)).into_response()
            }
            Some(WebhookResponse::Multipart { content_type, body }) => {
                let mut response = (StatusCode::OK, body).into_response();
                response.headers_mut().insert(
                    CONTENT_TYPE,
                    HeaderValue::from_str(&content_type).expect("generated content type is valid"),
                );
                response
            }
            None => StatusCode::OK.into_response(),
        }
    }

    async fn handle(
        State(state): State<WebhookState>,
        connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        if let Some(ip_filter) = &state.ip_filter {
            let forwarded_for = headers
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok());
            let peer_address =
                connect_info.and_then(|Extension(ConnectInfo(address))| match address.ip() {
                    IpAddr::V4(address) => Some(address),
                    IpAddr::V6(_) => None,
                });
            let (address, accepted) = ip_filter.check_client(forwarded_for, peer_address);
            if !accepted {
                tracing::warn!(%address, "blocking webhook request from unauthorized IP");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        }
        let provided = headers
            .get(SECRET_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok());
        if !verify_secret_token(provided, state.secret_token.as_deref()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if state.handle_in_background {
            return match feed_json_in_background(state.dispatcher, state.bot, &body) {
                Ok(_) => (StatusCode::OK, axum::Json(serde_json::json!({}))).into_response(),
                Err(error) => {
                    tracing::error!(%error, "invalid background webhook update");
                    StatusCode::BAD_REQUEST.into_response()
                }
            };
        }
        match feed_json_with_response(&state.dispatcher, state.bot, &body).await {
            Ok((_, response)) => method_response(response),
            Err(error) => {
                tracing::error!(%error, "webhook update failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }

    async fn handle_token(
        State(state): State<TokenWebhookState>,
        Path(bot_token): Path<String>,
        body: Bytes,
    ) -> Response {
        let bot = match state.bots.resolve(&bot_token) {
            Ok(bot) => bot,
            Err(error) => {
                tracing::warn!(%error, "invalid bot token in webhook path");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        };
        if state.handle_in_background {
            return match feed_json_in_background(state.dispatcher, bot, &body) {
                Ok(_) => (StatusCode::OK, axum::Json(serde_json::json!({}))).into_response(),
                Err(error) => {
                    tracing::error!(%error, "invalid background webhook update");
                    StatusCode::BAD_REQUEST.into_response()
                }
            };
        }
        match feed_json_with_response(&state.dispatcher, bot, &body).await {
            Ok((_, response)) => method_response(response),
            Err(error) => {
                tracing::error!(%error, "token webhook update failed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::Router;
    use crate::filters;
    use crate::methods::SendMessage;
    use crate::methods::SendPhoto;
    use crate::types::InputFile;

    #[cfg(feature = "webhook-axum")]
    use axum::body::{Body, to_bytes};
    #[cfg(feature = "webhook-axum")]
    use axum::http::{Request, StatusCode};
    #[cfg(feature = "webhook-axum")]
    use tower::ServiceExt;

    #[test]
    fn secret_token_is_optional_and_exact() {
        assert!(verify_secret_token(None, None));
        assert!(verify_secret_token(Some("secret"), Some("secret")));
        assert!(!verify_secret_token(None, Some("secret")));
        assert!(!verify_secret_token(Some("Secret"), Some("secret")));
    }

    #[test]
    fn webhook_ip_filter_and_token_registry_match_upstream_behavior() {
        let filter = IpFilter::telegram_default();
        assert!(filter.check("149.154.160.1"));
        assert!(!filter.check("149.154.159.255"));
        let (address, accepted) = filter.check_client(
            Some("91.108.4.10, 10.0.0.1"),
            Some("127.0.0.1".parse().unwrap()),
        );
        assert_eq!(address, "91.108.4.10");
        assert!(accepted);

        let registry = TokenBotRegistry::new();
        let token = "123456:abcdefghijklmnopqrstuvwxyzABCDE";
        assert_eq!(registry.resolve(token).unwrap().id(), 123456);
        assert_eq!(registry.resolve(token).unwrap().id(), 123456);
        assert_eq!(registry.len(), 1);
    }

    #[tokio::test]
    async fn webhook_can_dispatch_in_background() {
        let calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let mut router = Router::new();
        router.message(filters::any(), move |_| {
            let handler_calls = handler_calls.clone();
            async move {
                handler_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let body = serde_json::to_vec(&serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 42, "type": "private"},
                "text": "ping"
            }
        }))
        .unwrap();
        feed_json_in_background(
            Arc::new(dispatcher),
            Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
            &body,
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn handler_can_return_direct_webhook_method() {
        let mut router = Router::new();
        router.message(filters::any(), |context| async move {
            context.answer_webhook(&SendMessage::new(42_i64, "pong"))
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let body = serde_json::to_vec(&serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 42, "type": "private"},
                "text": "ping"
            }
        }))
        .unwrap();

        let (handled, response) = feed_json_with_response(
            &dispatcher,
            Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
            &body,
        )
        .await
        .unwrap();
        assert!(handled);
        let WebhookResponse::Json(response) = response.unwrap() else {
            panic!("expected JSON webhook response");
        };
        assert_eq!(response["method"], "sendMessage");
        assert_eq!(response["chat_id"], 42);
        assert_eq!(response["text"], "pong");
    }

    #[tokio::test]
    async fn handler_can_return_direct_multipart_webhook_method() {
        let mut router = Router::new();
        router.message(filters::any(), |context| async move {
            context.answer_webhook(&SendPhoto::new(
                42_i64,
                InputFile::named_bytes("photo_upload", "photo.jpg", b"PHOTO".to_vec()),
            ))
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let body = serde_json::to_vec(&serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 42, "type": "private"}
            }
        }))
        .unwrap();

        let (_, response) = feed_json_with_response(
            &dispatcher,
            Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
            &body,
        )
        .await
        .unwrap();
        let WebhookResponse::Multipart { content_type, body } = response.unwrap() else {
            panic!("expected multipart webhook response");
        };
        let body = String::from_utf8(body).unwrap();
        assert!(content_type.starts_with("multipart/form-data; boundary="));
        assert!(body.contains("name=\"method\"\r\n\r\nsendPhoto"));
        assert!(body.contains("name=\"photo_upload\"; filename=\"photo.jpg\""));
        assert!(body.contains("PHOTO"));
    }

    #[cfg(feature = "webhook-axum")]
    #[tokio::test]
    async fn axum_router_enforces_secret_and_ip_then_returns_direct_method() {
        let mut router = Router::new();
        router.message(filters::any(), |context| async move {
            context.answer_webhook(&SendMessage::new(42_i64, "pong"))
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let app = axum_integration::router_with_options(
            "/telegram",
            Arc::new(dispatcher),
            Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap(),
            axum_integration::WebhookOptions::new()
                .secret_token("secret")
                .ip_filter(IpFilter::new(["91.108.4.0/22"]).unwrap()),
        );
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 2,
                "date": 3,
                "chat": {"id": 42, "type": "private"},
                "text": "ping"
            }
        })
        .to_string();

        let unauthorized = app
            .clone()
            .oneshot(
                Request::post("/telegram")
                    .header(SECRET_TOKEN_HEADER, "wrong")
                    .header("x-forwarded-for", "91.108.4.10")
                    .body(Body::from(update.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let unauthorized_ip = app
            .clone()
            .oneshot(
                Request::post("/telegram")
                    .header(SECRET_TOKEN_HEADER, "secret")
                    .header("x-forwarded-for", "203.0.113.1")
                    .body(Body::from(update.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized_ip.status(), StatusCode::UNAUTHORIZED);

        let response = app
            .oneshot(
                Request::post("/telegram")
                    .header(SECRET_TOKEN_HEADER, "secret")
                    .header("x-forwarded-for", "91.108.4.10, 10.0.0.1")
                    .body(Body::from(update))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["method"], "sendMessage");
        assert_eq!(body["chat_id"], 42);
        assert_eq!(body["text"], "pong");
    }

    #[cfg(feature = "webhook-axum")]
    #[tokio::test]
    async fn axum_serve_handles_live_http_and_dispatcher_lifecycle() {
        let lifecycle = Arc::new(AtomicUsize::new(0));
        let mut router = Router::new();
        router.message(filters::any(), |context| async move {
            context.answer_webhook(&SendMessage::new(42_i64, "live"))
        });
        let mut dispatcher = Dispatcher::new();
        dispatcher.include_router(router);
        let calls = lifecycle.clone();
        dispatcher.startup(move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        let calls = lifecycle.clone();
        dispatcher.shutdown(move |_| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(10, Ordering::SeqCst);
                Ok(())
            }
        });
        let dispatcher = Arc::new(dispatcher);
        let bot = Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap();
        let app = axum_integration::router(
            "/telegram",
            dispatcher.clone(),
            bot.clone(),
            Some("secret".to_owned()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(axum_integration::serve(
            listener,
            app,
            dispatcher,
            bot,
            async move {
                let _ = shutdown_rx.await;
            },
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while lifecycle.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let response = reqwest::Client::new()
            .post(format!("http://{address}/telegram"))
            .header(SECRET_TOKEN_HEADER, "secret")
            .json(&serde_json::json!({
                "update_id": 1,
                "message": {
                    "message_id": 2,
                    "date": 3,
                    "chat": {"id": 42, "type": "private"},
                    "text": "ping"
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["method"], "sendMessage");
        assert_eq!(body["text"], "live");

        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(lifecycle.load(Ordering::SeqCst), 11);
    }
}
