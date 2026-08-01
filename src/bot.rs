use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::OnceCell;
use tokio_util::io::ReaderStream;

use crate::client::{
    BotRequest, DefaultBotProperties, RequestMiddleware, RequestNext, TelegramApiServer,
};
use crate::error::{Error, Result};
use crate::methods::{
    AnswerCallbackQuery, GetFile, GetMe, GetUpdates, SendCopyMethod, SendMessage, TelegramMethod,
    TelegramResponse,
};
use crate::types::{
    ChatId, Downloadable, File, InputFileContent, InputFileUpload, Message, Update, User,
};

const DEFAULT_PROPERTY_FIELDS: [&str; 6] = [
    "parse_mode",
    "disable_notification",
    "protect_content",
    "allow_sending_without_reply",
    "link_preview_options",
    "show_caption_above_media",
];

mod generated;
pub use generated::GENERATED_BOT_SHORTCUT_COUNT;

struct Inner {
    token: String,
    id: i64,
    api_server: TelegramApiServer,
    client: reqwest::Client,
    defaults: DefaultBotProperties,
    request_middlewares: Arc<Vec<Arc<dyn RequestMiddleware>>>,
    me: OnceCell<User>,
}

/// Cloneable asynchronous Telegram Bot API client.
#[derive(Clone)]
pub struct Bot {
    inner: Arc<Inner>,
}

impl PartialEq for Bot {
    fn eq(&self, other: &Self) -> bool {
        self.inner.token == other.inner.token
    }
}

impl Eq for Bot {}

impl Hash for Bot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.inner.token.hash(state);
    }
}

impl fmt::Debug for Bot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Bot")
            .field("token", &"<redacted>")
            .field("api_server", &self.inner.api_server)
            .finish()
    }
}

impl Bot {
    pub fn new(token: impl Into<String>) -> Result<Self> {
        Self::builder(token).build()
    }

    /// Creates a bot against a custom Bot API server or a local test server.
    pub fn with_api_base(token: impl Into<String>, api_base: impl Into<String>) -> Result<Self> {
        Self::builder(token).api_base(api_base).build()
    }

    pub fn with_api_server(token: impl Into<String>, server: TelegramApiServer) -> Result<Self> {
        Self::builder(token).api_server(server).build()
    }

    pub fn builder(token: impl Into<String>) -> BotBuilder {
        BotBuilder {
            token: token.into(),
            api_server: TelegramApiServer::production(),
            defaults: DefaultBotProperties::default(),
            client: None,
            request_middlewares: Vec::new(),
        }
    }

    pub fn default_properties(&self) -> &DefaultBotProperties {
        &self.inner.defaults
    }

    pub fn id(&self) -> i64 {
        self.inner.id
    }

    pub fn token(&self) -> &str {
        &self.inner.token
    }

    pub fn api_server(&self) -> &TelegramApiServer {
        &self.inner.api_server
    }

    pub async fn execute<M: TelegramMethod>(&self, method: &M) -> Result<M::Response> {
        let request = self.prepare_request(method)?;
        let response = self.dispatch_request(request).await?;
        serde_json::from_value(response.clone()).map_err(|error| Error::ClientDecode {
            method: M::NAME.to_owned(),
            reason: error.to_string(),
            data: response.to_string(),
        })
    }

    /// Executes a method without applying any [`DefaultBotProperties`].
    pub async fn execute_without_defaults<M: TelegramMethod>(
        &self,
        method: &M,
    ) -> Result<M::Response> {
        let request = self.prepare_request_suppressing_defaults(method, DEFAULT_PROPERTY_FIELDS)?;
        let response = self.dispatch_request(request).await?;
        serde_json::from_value(response.clone()).map_err(|error| Error::ClientDecode {
            method: M::NAME.to_owned(),
            reason: error.to_string(),
            data: response.to_string(),
        })
    }

    /// Suppresses selected client defaults only when the method did not set an
    /// explicit value for the same field.
    pub async fn execute_suppressing_defaults<M, I, S>(
        &self,
        method: &M,
        fields: I,
    ) -> Result<M::Response>
    where
        M: TelegramMethod,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let request = self.prepare_request_suppressing_defaults(method, fields)?;
        let response = self.dispatch_request(request).await?;
        serde_json::from_value(response.clone()).map_err(|error| Error::ClientDecode {
            method: M::NAME.to_owned(),
            reason: error.to_string(),
            data: response.to_string(),
        })
    }

    pub async fn execute_with_timeout<M: TelegramMethod>(
        &self,
        method: &M,
        timeout: Duration,
    ) -> Result<M::Response> {
        match tokio::time::timeout(timeout, self.execute(method)).await {
            Ok(result) => result,
            Err(_) => Err(Error::RequestTimeout {
                method: M::NAME.to_owned(),
                timeout,
            }),
        }
    }

    pub(crate) fn prepare_request<M: TelegramMethod>(&self, method: &M) -> Result<BotRequest> {
        let mut payload = serde_json::to_value(method)?;
        self.inner
            .defaults
            .apply(M::DEFAULT_PROPERTIES, &mut payload)?;
        Ok(BotRequest::new(
            M::NAME,
            payload,
            deduplicate_files(method.files())?,
        ))
    }

    fn prepare_request_suppressing_defaults<M, I, S>(
        &self,
        method: &M,
        fields: I,
    ) -> Result<BotRequest>
    where
        M: TelegramMethod,
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut payload = serde_json::to_value(method)?;
        let object = payload.as_object_mut().ok_or_else(|| {
            Error::InvalidPayload("Telegram method payload must be an object".to_owned())
        })?;
        let fields: BTreeSet<String> = fields
            .into_iter()
            .map(|field| field.as_ref().to_owned())
            .collect();
        for &(wire_field, property) in M::DEFAULT_PROPERTIES {
            if (fields.contains(wire_field) || fields.contains(property))
                && !object.contains_key(wire_field)
            {
                object.insert(wire_field.to_owned(), serde_json::Value::Null);
            }
        }
        self.inner
            .defaults
            .apply(M::DEFAULT_PROPERTIES, &mut payload)?;
        Ok(BotRequest::new(
            M::NAME,
            payload,
            deduplicate_files(method.files())?,
        ))
    }

    /// Executes any Telegram method with a serializable payload.
    ///
    /// This is the forward-compatibility escape hatch for Bot API methods that
    /// are newer than the schema pinned by this crate.
    pub async fn request<T, R>(&self, method_name: &str, payload: &T) -> Result<R>
    where
        T: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let payload = serde_json::to_value(payload)?;
        let response = self
            .dispatch_request(BotRequest::new(method_name, payload, Vec::new()))
            .await?;
        serde_json::from_value(response.clone()).map_err(|error| Error::ClientDecode {
            method: method_name.to_owned(),
            reason: error.to_string(),
            data: response.to_string(),
        })
    }

    async fn dispatch_request(&self, request: BotRequest) -> Result<serde_json::Value> {
        RequestNext::new(self.inner.request_middlewares.clone())
            .run(self.clone(), request)
            .await
    }

    pub(crate) async fn send_request(&self, request: BotRequest) -> Result<serde_json::Value> {
        if request.files.is_empty() {
            self.request_value(&request.method_name, &request.payload)
                .await
        } else {
            self.request_multipart(&request.method_name, &request.payload, request.files)
                .await
        }
    }

    async fn request_value(
        &self,
        method_name: &str,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = self
            .inner
            .api_server
            .api_url(&self.inner.token, method_name);
        let response = self.inner.client.post(url).json(payload).send().await?;
        self.decode_response(method_name, response).await
    }

    async fn request_multipart(
        &self,
        method_name: &str,
        payload: &serde_json::Value,
        files: Vec<InputFileUpload>,
    ) -> Result<serde_json::Value> {
        let object = payload.as_object().ok_or_else(|| {
            Error::InvalidPayload("Telegram method payload must be an object".to_owned())
        })?;
        let mut form = reqwest::multipart::Form::new();
        for (name, value) in object {
            if value.is_null() {
                continue;
            }
            let text = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.to_string(),
                _ => serde_json::to_string(value)?,
            };
            form = form.text(name.clone(), text);
        }
        for file in files {
            let part = match file.content {
                InputFileContent::Bytes(data) => {
                    reqwest::multipart::Part::bytes(data.as_ref().to_vec())
                }
                InputFileContent::Path(path) => {
                    let file = tokio::fs::File::open(&path).await?;
                    let length = file.metadata().await?.len();
                    reqwest::multipart::Part::stream_with_length(
                        reqwest::Body::wrap_stream(ReaderStream::new(file)),
                        length,
                    )
                }
                InputFileContent::Url {
                    url,
                    headers,
                    timeout,
                } => {
                    let mut request = self.inner.client.get(url).timeout(timeout);
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                    let response = request.send().await?.error_for_status()?;
                    match response.content_length() {
                        Some(length) => reqwest::multipart::Part::stream_with_length(
                            reqwest::Body::wrap_stream(response.bytes_stream()),
                            length,
                        ),
                        None => reqwest::multipart::Part::stream(reqwest::Body::wrap_stream(
                            response.bytes_stream(),
                        )),
                    }
                }
            }
            .file_name(file.file_name);
            form = form.part(file.attachment_name, part);
        }

        let url = self
            .inner
            .api_server
            .api_url(&self.inner.token, method_name);
        let response = self.inner.client.post(url).multipart(form).send().await?;
        self.decode_response(method_name, response).await
    }

    pub(crate) async fn load_input_file(&self, content: InputFileContent) -> Result<Vec<u8>> {
        match content {
            InputFileContent::Bytes(data) => Ok(data.as_ref().to_vec()),
            InputFileContent::Path(path) => Ok(tokio::fs::read(path).await?),
            InputFileContent::Url {
                url,
                headers,
                timeout,
            } => {
                let mut request = self.inner.client.get(url).timeout(timeout);
                for (name, value) in headers {
                    request = request.header(name, value);
                }
                Ok(request
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?
                    .to_vec())
            }
        }
    }

    async fn decode_response<R: DeserializeOwned>(
        &self,
        method_name: &str,
        response: reqwest::Response,
    ) -> Result<R> {
        let status = response.status();
        let content = response.text().await?;
        let body: TelegramResponse<R> =
            serde_json::from_str(&content).map_err(|error| Error::ClientDecode {
                method: method_name.to_owned(),
                reason: error.to_string(),
                data: content,
            })?;

        if (200..=226).contains(&status.as_u16()) && body.ok {
            return body.result.ok_or_else(|| Error::Telegram {
                method: method_name.to_owned(),
                error_code: status.as_u16(),
                description: "successful Telegram response did not contain result".to_owned(),
                parameters: body.parameters,
            });
        }

        let description = body.description.unwrap_or_else(|| status.to_string());
        if let Some(retry_after) = body.parameters.as_ref().and_then(|p| p.retry_after) {
            return Err(Error::RetryAfter {
                method: method_name.to_owned(),
                retry_after: Duration::from_secs(retry_after),
                description,
            });
        }
        if let Some(migrate_to_chat_id) = body
            .parameters
            .as_ref()
            .and_then(|parameters| parameters.migrate_to_chat_id)
        {
            return Err(Error::MigrateToChat {
                method: method_name.to_owned(),
                migrate_to_chat_id,
                description,
            });
        }

        let method = method_name.to_owned();
        match status.as_u16() {
            400 => {
                return Err(Error::BadRequest {
                    method,
                    description,
                });
            }
            401 => {
                return Err(Error::Unauthorized {
                    method,
                    description,
                });
            }
            403 => {
                return Err(Error::Forbidden {
                    method,
                    description,
                });
            }
            404 => {
                return Err(Error::NotFound {
                    method,
                    description,
                });
            }
            409 => {
                return Err(Error::Conflict {
                    method,
                    description,
                });
            }
            413 => {
                return Err(Error::EntityTooLarge {
                    method,
                    description,
                });
            }
            500..=599 if description.contains("restart") => {
                return Err(Error::Restarting {
                    method,
                    description,
                });
            }
            500..=599 => {
                return Err(Error::Server {
                    method,
                    description,
                });
            }
            _ => {}
        }

        Err(Error::Telegram {
            method,
            error_code: body.error_code.unwrap_or(status.as_u16()),
            description,
            parameters: body.parameters,
        })
    }

    pub async fn get_me(&self) -> Result<User> {
        self.inner
            .me
            .get_or_try_init(|| async { self.execute(&GetMe::default()).await })
            .await
            .cloned()
    }

    pub async fn get_updates(&self, method: GetUpdates) -> Result<Vec<Update>> {
        self.execute(&method).await
    }

    pub async fn get_file(&self, file_id: impl Into<String>) -> Result<File> {
        self.execute(&GetFile::new(file_id)).await
    }

    /// Downloads a file using a path returned by Telegram's `getFile` method.
    pub async fn download_file(&self, file_path: &str) -> Result<Vec<u8>> {
        if self.inner.api_server.is_local {
            return Ok(tokio::fs::read(self.inner.api_server.to_local(file_path)?).await?);
        }
        let url = self.inner.api_server.file_url(&self.inner.token, file_path);
        let response = self
            .inner
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.bytes().await?.to_vec())
    }

    /// Downloads any Telegram object with a `file_id`, resolving its current
    /// file path through `getFile` first, like aiogram's `Bot.download`.
    pub async fn download<D: Downloadable + ?Sized>(&self, file: &D) -> Result<Vec<u8>> {
        self.download_by_id(file.file_id()).await
    }

    pub async fn download_by_id(&self, file_id: &str) -> Result<Vec<u8>> {
        let file = self.get_file(file_id).await?;
        self.download_descriptor(&file).await
    }

    /// Downloads a previously resolved Telegram `File` descriptor.
    pub async fn download_descriptor(&self, file: &File) -> Result<Vec<u8>> {
        let path = file.file_path.as_deref().ok_or_else(|| {
            Error::InvalidPayload("Telegram File does not contain file_path".to_owned())
        })?;
        self.download_file(path).await
    }

    pub async fn download_to<D, W>(&self, file: &D, writer: &mut W) -> Result<u64>
    where
        D: Downloadable + ?Sized,
        W: AsyncWrite + Unpin + Send,
    {
        let file = self.get_file(file.file_id()).await?;
        let path = file.file_path.as_deref().ok_or_else(|| {
            Error::InvalidPayload("Telegram File does not contain file_path".to_owned())
        })?;
        self.download_file_to(path, writer).await
    }

    pub async fn download_file_to<W>(&self, file_path: &str, writer: &mut W) -> Result<u64>
    where
        W: AsyncWrite + Unpin + Send,
    {
        if self.inner.api_server.is_local {
            let mut file =
                tokio::fs::File::open(self.inner.api_server.to_local(file_path)?).await?;
            return Ok(tokio::io::copy(&mut file, writer).await?);
        }
        let url = self.inner.api_server.file_url(&self.inner.token, file_path);
        let response = self
            .inner
            .client
            .get(url)
            .send()
            .await?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut written = 0_u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            writer.write_all(&chunk).await?;
            written += chunk.len() as u64;
        }
        Ok(written)
    }

    pub async fn send_message(
        &self,
        chat_id: impl Into<ChatId>,
        text: impl Into<String>,
    ) -> Result<Message> {
        self.execute(&SendMessage::new(chat_id, text)).await
    }

    /// Executes the concrete request selected by [`Message::send_copy`](crate::types::Message::send_copy).
    pub async fn execute_send_copy(&self, method: &SendCopyMethod) -> Result<Message> {
        match method {
            SendCopyMethod::ForwardMessage(method) => self.execute(method).await,
            SendCopyMethod::SendAnimation(method) => self.execute(method).await,
            SendCopyMethod::SendAudio(method) => self.execute(method).await,
            SendCopyMethod::SendContact(method) => self.execute(method).await,
            SendCopyMethod::SendDocument(method) => self.execute(method).await,
            SendCopyMethod::SendLocation(method) => self.execute(method).await,
            SendCopyMethod::SendMessage(method) => self.execute(method).await,
            SendCopyMethod::SendPhoto(method) => self.execute(method).await,
            SendCopyMethod::SendPoll(method) => self.execute(method).await,
            SendCopyMethod::SendDice(method) => self.execute(method).await,
            SendCopyMethod::SendSticker(method) => self.execute(method).await,
            SendCopyMethod::SendVenue(method) => self.execute(method).await,
            SendCopyMethod::SendVideo(method) => self.execute(method).await,
            SendCopyMethod::SendVideoNote(method) => self.execute(method).await,
            SendCopyMethod::SendVoice(method) => self.execute(method).await,
        }
    }

    pub async fn answer_callback_query(
        &self,
        callback_query_id: impl Into<String>,
    ) -> Result<bool> {
        self.execute(&AnswerCallbackQuery::new(callback_query_id))
            .await
    }
}

/// Configures a [`Bot`] with a custom API base and aiogram-style defaults.
pub struct BotBuilder {
    token: String,
    api_server: TelegramApiServer,
    defaults: DefaultBotProperties,
    client: Option<reqwest::Client>,
    request_middlewares: Vec<Arc<dyn RequestMiddleware>>,
}

impl BotBuilder {
    pub fn api_base(mut self, value: impl Into<String>) -> Self {
        self.api_server = TelegramApiServer::from_base(value.into());
        self
    }

    pub fn api_server(mut self, value: TelegramApiServer) -> Self {
        self.api_server = value;
        self
    }

    pub fn defaults(mut self, value: DefaultBotProperties) -> Self {
        self.defaults = value;
        self
    }

    /// Uses a preconfigured reqwest client (proxy, timeout, connector, etc.).
    pub fn client(mut self, value: reqwest::Client) -> Self {
        self.client = Some(value);
        self
    }

    pub fn request_middleware(mut self, value: impl RequestMiddleware + 'static) -> Self {
        self.request_middlewares.push(Arc::new(value));
        self
    }

    pub fn build(self) -> Result<Bot> {
        validate_token(&self.token)?;
        let id = crate::utils::token::extract_bot_id(&self.token)?;
        let client = match self.client {
            Some(client) => client,
            None => reqwest::Client::builder()
                .user_agent(concat!("aiogram-rust/", env!("CARGO_PKG_VERSION")))
                .build()?,
        };
        Ok(Bot {
            inner: Arc::new(Inner {
                token: self.token,
                id,
                api_server: self.api_server,
                client,
                defaults: self.defaults,
                request_middlewares: Arc::new(self.request_middlewares),
                me: OnceCell::new(),
            }),
        })
    }
}

fn validate_token(token: &str) -> Result<()> {
    crate::utils::token::validate(token)
}

fn deduplicate_files(files: Vec<InputFileUpload>) -> Result<Vec<InputFileUpload>> {
    let mut unique = BTreeMap::<String, InputFileUpload>::new();
    for file in files {
        match unique.get(&file.attachment_name) {
            Some(existing) if existing == &file => {}
            Some(_) => {
                return Err(Error::InvalidPayload(format!(
                    "multipart attachment name {:?} refers to different files",
                    file.attachment_name
                )));
            }
            None => {
                unique.insert(file.attachment_name.clone(), file);
            }
        }
    }
    Ok(unique.into_values().collect())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::methods::{SendDocument, SendPhoto};
    use crate::types::InputFile;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn rejects_invalid_token() {
        assert!(matches!(
            Bot::new("not-a-token"),
            Err(Error::InvalidToken(_))
        ));
    }

    #[test]
    fn generated_shortcuts_cover_non_core_bot_api_methods() {
        assert_eq!(GENERATED_BOT_SHORTCUT_COUNT, 180);
    }

    #[test]
    fn rejects_conflicting_multipart_attachment_names() {
        let bot = bot_for_tests();
        let method = SendDocument::new(
            3_i64,
            InputFile::named_bytes("same", "document.txt", b"DOCUMENT".to_vec()),
        )
        .thumbnail(InputFile::named_bytes(
            "same",
            "thumbnail.jpg",
            b"THUMBNAIL".to_vec(),
        ));
        assert!(matches!(
            bot.prepare_request(&method),
            Err(Error::InvalidPayload(message)) if message.contains("same")
        ));
    }

    #[test]
    fn debug_never_exposes_token() {
        let token = "123456:abcdefghijklmnopqrstuvwxyzABCDE";
        let bot = Bot::new(token).unwrap();
        let output = format!("{bot:?}");
        assert!(!output.contains(token));
        assert!(output.contains("<redacted>"));
    }

    #[test]
    fn equality_and_hash_follow_the_bot_token() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash(bot: &Bot) -> u64 {
            let mut hasher = DefaultHasher::new();
            bot.hash(&mut hasher);
            hasher.finish()
        }

        let production = Bot::new("42:secret").unwrap();
        let custom_server = Bot::with_api_base("42:secret", "http://localhost:8081").unwrap();
        let other = Bot::new("43:secret").unwrap();

        assert_eq!(production, custom_server);
        assert_ne!(production, other);
        assert_eq!(hash(&production), hash(&custom_server));
        assert_ne!(hash(&production), hash(&other));
    }

    #[test]
    fn send_copy_explicit_none_suppresses_bot_defaults() {
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .defaults(
                DefaultBotProperties::default()
                    .parse_mode("HTML")
                    .disable_notification(true)
                    .allow_sending_without_reply(true)
                    .link_preview_is_disabled(true),
            )
            .build()
            .unwrap();
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 7,
            "date": 1_700_000_000,
            "chat": {"id": 10, "type": "private"},
            "text": "already formatted",
            "entities": [{"type":"bold","offset":0,"length":7}]
        }))
        .unwrap();
        let method = message.send_copy(42_i64).unwrap();
        let SendCopyMethod::SendMessage(method) = method else {
            panic!("text copy must select SendMessage");
        };
        let payload = bot.prepare_request(&method).unwrap().payload;
        assert!(payload.get("parse_mode").is_none());
        assert!(payload.get("disable_notification").is_none());
        assert!(payload.get("allow_sending_without_reply").is_none());
        assert!(payload.get("link_preview_options").is_none());
        assert_eq!(payload["entities"][0]["type"], "bold");
    }

    fn bot_for_tests() -> Bot {
        Bot::new("123456:abcdefghijklmnopqrstuvwxyzABCDE").unwrap()
    }

    async fn mock_server(
        response_status: &str,
        response_json: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = response_status.to_owned();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 8192];
            let mut expected_len = None;
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if expected_len.is_none()
                    && let Some(header_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    expected_len = Some(header_end + 4 + content_length);
                }
                if expected_len.is_some_and(|length| request.len() >= length) {
                    break;
                }
            }
            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response_json}",
                response_json.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&request).into_owned()
        });
        (format!("http://{address}"), task)
    }

    async fn download_mock_server() -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let responses = [
                (
                    "application/json",
                    r#"{"ok":true,"result":{"file_id":"file-id","file_unique_id":"unique-id","file_path":"photos/avatar.jpg"}}"#,
                ),
                ("application/octet-stream", "DOWNLOADABLE_BYTES"),
            ];
            let mut requests = Vec::new();
            for (content_type, body) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 8192];
                let mut expected_len = None;
                loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if expected_len.is_none()
                        && let Some(header_end) =
                            request.windows(4).position(|part| part == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                        expected_len = Some(header_end + 4 + content_length);
                    }
                    if expected_len.is_some_and(|length| request.len() >= length) {
                        break;
                    }
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                requests.push(String::from_utf8_lossy(&request).into_owned());
            }
            requests
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn executes_json_method_and_deserializes_response() {
        let (api_base, request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"id":123,"is_bot":true,"first_name":"Rust bot"}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();

        let user = bot.get_me().await.unwrap();
        assert_eq!(user.id, 123);
        assert_eq!(bot.get_me().await.unwrap().id, 123);
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /bot123456:abcdefghijklmnopqrstuvwxyzABCDE/getMe "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
    }

    #[tokio::test]
    async fn applies_defaults_only_to_supported_method_fields() {
        let (api_base, request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"message_id":1,"date":2,"chat":{"id":3,"type":"private"}}}"#,
        )
        .await;
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .api_base(api_base)
            .defaults(
                DefaultBotProperties::default()
                    .parse_mode("HTML")
                    .protect_content(true)
                    .link_preview_is_disabled(true),
            )
            .build()
            .unwrap();

        bot.send_message(3_i64, "hello").await.unwrap();
        let request = request.await.unwrap();
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["parse_mode"], "HTML");
        assert_eq!(payload["protect_content"], true);
        assert_eq!(payload["link_preview_options"]["is_disabled"], true);
        assert!(payload.get("disable_notification").is_none());
    }

    #[test]
    fn generated_default_mappings_cover_aliases_and_exclude_plain_optional_fields() {
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .defaults(
                DefaultBotProperties::default()
                    .parse_mode("HTML")
                    .protect_content(true),
            )
            .build()
            .unwrap();
        let poll = crate::methods::SendPoll::new(42_i64, "Question", Vec::new());
        let payload = bot.prepare_request(&poll).unwrap().payload;
        assert_eq!(payload["question_parse_mode"], "HTML");
        assert_eq!(payload["explanation_parse_mode"], "HTML");
        assert_eq!(payload["description_parse_mode"], "HTML");
        assert_eq!(payload["protect_content"], true);

        let ephemeral = crate::methods::EditEphemeralMessageText::new(42_i64, 7, 8, "Text");
        let payload = bot.prepare_request(&ephemeral).unwrap().payload;
        assert!(payload.get("parse_mode").is_none());
        assert!(payload.get("protect_content").is_none());
    }

    #[tokio::test]
    async fn call_can_suppress_selected_or_all_bot_defaults() {
        let response =
            r#"{"ok":true,"result":{"message_id":1,"date":2,"chat":{"id":3,"type":"private"}}}"#;
        let (api_base, request) = mock_server("200 OK", response).await;
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .api_base(api_base)
            .defaults(
                DefaultBotProperties::default()
                    .parse_mode("HTML")
                    .protect_content(true),
            )
            .build()
            .unwrap();
        bot.execute_suppressing_defaults(&SendMessage::new(3_i64, "hello"), ["parse_mode"])
            .await
            .unwrap();
        let request = request.await.unwrap();
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body.get("parse_mode").is_none());
        assert_eq!(body["protect_content"], true);

        let (api_base, request) = mock_server("200 OK", response).await;
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .api_base(api_base)
            .defaults(
                DefaultBotProperties::default()
                    .parse_mode("HTML")
                    .protect_content(true),
            )
            .build()
            .unwrap();
        bot.execute_without_defaults(&SendMessage::new(3_i64, "hello"))
            .await
            .unwrap();
        let request = request.await.unwrap();
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body.get("parse_mode").is_none());
        assert!(body.get("protect_content").is_none());
        assert_eq!(body["text"], "hello");
    }

    #[tokio::test]
    async fn per_call_timeout_cancels_a_stalled_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
        });
        let bot = Bot::with_api_base(
            "123456:abcdefghijklmnopqrstuvwxyzABCDE",
            format!("http://{address}"),
        )
        .unwrap();
        assert!(matches!(
            bot.execute_with_timeout(&GetMe::default(), Duration::from_millis(10))
                .await
                .unwrap_err(),
            Error::RequestTimeout { method, .. } if method == "getMe"
        ));
        server.abort();
    }

    #[tokio::test]
    async fn malformed_or_wrongly_typed_success_response_is_a_client_decode_error() {
        let (api_base, _request) = mock_server("200 OK", "not-json").await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        assert!(matches!(
            bot.get_me().await.unwrap_err(),
            Error::ClientDecode { method, data, .. }
                if method == "getMe" && data == "not-json"
        ));

        let (api_base, _request) = mock_server("200 OK", r#"{"ok":true,"result":true}"#).await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        assert!(matches!(
            bot.get_me().await.unwrap_err(),
            Error::ClientDecode { method, .. } if method == "getMe"
        ));
    }

    #[tokio::test]
    async fn ok_body_on_http_error_is_not_accepted_as_success() {
        let (api_base, _request) = mock_server(
            "500 Internal Server Error",
            r#"{"ok":true,"result":{"id":123,"is_bot":true,"first_name":"Bot"}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        assert!(matches!(
            bot.get_me().await.unwrap_err(),
            Error::Server { method, .. } if method == "getMe"
        ));
    }

    #[tokio::test]
    async fn turns_retry_after_into_typed_error() {
        let (api_base, _request) = mock_server(
            "429 Too Many Requests",
            r#"{"ok":false,"error_code":429,"description":"flood control","parameters":{"retry_after":3}}"#,
        ).await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();

        let error = bot.get_me().await.unwrap_err();
        assert!(matches!(
            error,
            Error::RetryAfter { retry_after, .. } if retry_after == Duration::from_secs(3)
        ));
    }

    #[tokio::test]
    async fn classifies_all_upstream_telegram_http_error_categories() {
        let cases = [
            ("400 Bad Request", "bad_request"),
            ("401 Unauthorized", "unauthorized"),
            ("403 Forbidden", "forbidden"),
            ("404 Not Found", "not_found"),
            ("409 Conflict", "conflict"),
            ("413 Payload Too Large", "entity_too_large"),
            ("500 Internal Server Error", "server"),
        ];
        for (status, expected) in cases {
            let (api_base, _request) = mock_server(
                status,
                r#"{"ok":false,"error_code":500,"description":"request failed"}"#,
            )
            .await;
            let bot =
                Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
            let error = bot.get_me().await.unwrap_err();
            let (actual, method) = match error {
                Error::BadRequest { method, .. } => ("bad_request", method),
                Error::Unauthorized { method, .. } => ("unauthorized", method),
                Error::Forbidden { method, .. } => ("forbidden", method),
                Error::NotFound { method, .. } => ("not_found", method),
                Error::Conflict { method, .. } => ("conflict", method),
                Error::EntityTooLarge { method, .. } => ("entity_too_large", method),
                Error::Server { method, .. } => ("server", method),
                error => panic!("unexpected error classification: {error:?}"),
            };
            assert_eq!(actual, expected);
            assert_eq!(method, "getMe");
        }

        let (api_base, _request) = mock_server(
            "503 Service Unavailable",
            r#"{"ok":false,"error_code":503,"description":"server restart in progress"}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        assert!(matches!(
            bot.get_me().await.unwrap_err(),
            Error::Restarting { method, .. } if method == "getMe"
        ));
    }

    #[tokio::test]
    async fn migrate_to_chat_parameter_takes_precedence_over_http_status() {
        let (api_base, _request) = mock_server(
            "400 Bad Request",
            r#"{"ok":false,"error_code":400,"description":"migrated","parameters":{"migrate_to_chat_id":-100123}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        assert!(matches!(
            bot.get_me().await.unwrap_err(),
            Error::MigrateToChat {
                method,
                migrate_to_chat_id: -100123,
                ..
            } if method == "getMe"
        ));
    }

    #[tokio::test]
    async fn executes_file_method_as_multipart() {
        let (api_base, request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"message_id":1,"date":2,"chat":{"id":3,"type":"private"}}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        let message = bot
            .send_photo(
                ChatId::Id(3),
                InputFile::named_bytes("photo_upload", "avatar.jpg", b"IMAGE_BYTES".to_vec()),
            )
            .await
            .unwrap();
        assert_eq!(message.message_id, 1);
        let request = request.await.unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: multipart/form-data")
        );
        assert!(request.contains("attach://photo_upload"));
        assert!(request.contains("filename=\"avatar.jpg\""));
        assert!(request.contains("IMAGE_BYTES"));
    }

    #[tokio::test]
    async fn lazily_reads_filesystem_uploads() {
        let path = std::env::temp_dir().join(format!(
            "aiogram-rust-upload-test-{}.txt",
            std::process::id()
        ));
        std::fs::write(&path, b"FILESYSTEM_BYTES").unwrap();
        let (api_base, request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"message_id":1,"date":2,"chat":{"id":3,"type":"private"}}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        let method = SendPhoto::new(
            3_i64,
            InputFile::named_path("disk_upload", "document.txt", &path),
        );

        bot.execute(&method).await.unwrap();
        let request = request.await.unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(request.contains("filename=\"document.txt\""));
        assert!(request.contains("FILESYSTEM_BYTES"));
    }

    #[tokio::test]
    async fn downloads_url_input_file_with_bot_http_client() {
        let (file_url, file_request) = mock_server("200 OK", "REMOTE_FILE_BYTES").await;
        let (api_base, api_request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"message_id":1,"date":2,"chat":{"id":3,"type":"private"}}}"#,
        )
        .await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();

        bot.send_photo(3_i64, InputFile::url("remote.jpg", file_url))
            .await
            .unwrap();

        assert!(file_request.await.unwrap().starts_with("GET / "));
        let request = api_request.await.unwrap();
        assert!(request.contains("filename=\"remote.jpg\""));
        assert!(request.contains("REMOTE_FILE_BYTES"));
    }

    struct InjectRequestField(Arc<AtomicUsize>);

    #[async_trait::async_trait]
    impl RequestMiddleware for InjectRequestField {
        async fn handle(
            &self,
            bot: Bot,
            mut request: BotRequest,
            next: RequestNext,
        ) -> Result<serde_json::Value> {
            self.0.fetch_add(1, Ordering::SeqCst);
            request.payload["middleware"] = serde_json::json!("visited");
            next.run(bot, request).await
        }
    }

    #[tokio::test]
    async fn request_middleware_wraps_typed_execution() {
        let (api_base, request) = mock_server(
            "200 OK",
            r#"{"ok":true,"result":{"id":123,"is_bot":true,"first_name":"Rust bot"}}"#,
        )
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let bot = Bot::builder("123456:abcdefghijklmnopqrstuvwxyzABCDE")
            .api_base(api_base)
            .request_middleware(InjectRequestField(calls.clone()))
            .build()
            .unwrap();

        bot.get_me().await.unwrap();
        let request = request.await.unwrap();
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["middleware"], "visited");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn downloads_file_from_telegram_file_endpoint() {
        let (api_base, request) = mock_server("200 OK", "FILE_BYTES").await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();

        assert_eq!(
            bot.download_file("photos/avatar.jpg").await.unwrap(),
            b"FILE_BYTES"
        );
        assert!(
            request.await.unwrap().starts_with(
                "GET /file/bot123456:abcdefghijklmnopqrstuvwxyzABCDE/photos/avatar.jpg "
            )
        );
    }

    #[tokio::test]
    async fn downloadable_resolves_get_file_then_downloads_the_current_path() {
        let (api_base, requests) = download_mock_server().await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        let photo = crate::types::PhotoSize::new("file-id", "unique-id", 320, 240);

        assert_eq!(bot.download(&photo).await.unwrap(), b"DOWNLOADABLE_BYTES");
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0].starts_with("POST /bot123456:abcdefghijklmnopqrstuvwxyzABCDE/getFile ")
        );
        let body: serde_json::Value =
            serde_json::from_str(requests[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["file_id"], "file-id");
        assert!(
            requests[1].starts_with(
                "GET /file/bot123456:abcdefghijklmnopqrstuvwxyzABCDE/photos/avatar.jpg "
            )
        );
    }

    #[tokio::test]
    async fn streams_download_into_async_writer() {
        let (api_base, request) = mock_server("200 OK", "STREAMED_FILE_BYTES").await;
        let bot = Bot::with_api_base("123456:abcdefghijklmnopqrstuvwxyzABCDE", api_base).unwrap();
        let (mut writer, mut reader) = tokio::io::duplex(64);

        let written = bot
            .download_file_to("document.bin", &mut writer)
            .await
            .unwrap();
        drop(writer);
        let mut content = Vec::new();
        reader.read_to_end(&mut content).await.unwrap();

        assert_eq!(written, 19);
        assert_eq!(content, b"STREAMED_FILE_BYTES");
        assert!(request.await.unwrap().contains("/document.bin "));
    }

    #[tokio::test]
    async fn local_api_server_download_reads_mapped_file_without_http() {
        let directory = std::env::temp_dir().join(format!(
            "aiogram-rust-local-api-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = directory.join("photos/avatar.jpg");
        tokio::fs::create_dir_all(file.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&file, b"LOCAL_FILE").await.unwrap();
        let server = TelegramApiServer::from_base("http://127.0.0.1:1")
            .local_file_paths("/telegram", &directory);
        let bot = Bot::with_api_server("123456:abcdefghijklmnopqrstuvwxyzABCDE", server).unwrap();
        assert_eq!(
            bot.download_file("/telegram/photos/avatar.jpg")
                .await
                .unwrap(),
            b"LOCAL_FILE"
        );
        tokio::fs::remove_dir_all(directory).await.unwrap();
    }
}
