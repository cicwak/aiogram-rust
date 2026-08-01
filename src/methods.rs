//! Typed Telegram Bot API method payloads generated from upstream metadata.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::types::{CollectFiles, InputFileUpload};

mod generated;
pub use generated::*;

/// Optional arguments shared by [`crate::types::Message::send_copy_with_options`].
#[derive(Debug, Clone, Default)]
pub struct SendCopyOptions {
    pub disable_notification: Option<bool>,
    pub reply_to_message_id: Option<i64>,
    pub reply_parameters: Option<crate::types::ReplyParameters>,
    pub reply_markup: Option<crate::types::ReplyMarkupUnion>,
    pub allow_sending_without_reply: Option<bool>,
    pub message_thread_id: Option<i64>,
    pub business_connection_id: Option<String>,
    pub parse_mode: Option<String>,
    pub message_effect_id: Option<String>,
    pub link_preview_options: Option<crate::types::LinkPreviewOptions>,
}

impl SendCopyOptions {
    pub fn disable_notification(mut self, value: bool) -> Self {
        self.disable_notification = Some(value);
        self
    }

    pub fn reply_to_message_id(mut self, value: i64) -> Self {
        self.reply_to_message_id = Some(value);
        self
    }

    pub fn reply_parameters(mut self, value: crate::types::ReplyParameters) -> Self {
        self.reply_parameters = Some(value);
        self
    }

    pub fn reply_markup(mut self, value: impl Into<crate::types::ReplyMarkupUnion>) -> Self {
        self.reply_markup = Some(value.into());
        self
    }

    pub fn allow_sending_without_reply(mut self, value: bool) -> Self {
        self.allow_sending_without_reply = Some(value);
        self
    }

    pub fn message_thread_id(mut self, value: i64) -> Self {
        self.message_thread_id = Some(value);
        self
    }

    pub fn business_connection_id(mut self, value: impl Into<String>) -> Self {
        self.business_connection_id = Some(value.into());
        self
    }

    pub fn parse_mode(mut self, value: impl Into<String>) -> Self {
        self.parse_mode = Some(value.into());
        self
    }

    pub fn message_effect_id(mut self, value: impl Into<String>) -> Self {
        self.message_effect_id = Some(value.into());
        self
    }

    pub fn link_preview_options(mut self, value: crate::types::LinkPreviewOptions) -> Self {
        self.link_preview_options = Some(value);
        self
    }
}

/// Concrete Telegram request selected for aiogram-compatible message copying.
#[derive(Debug, Clone)]
pub enum SendCopyMethod {
    ForwardMessage(ForwardMessage),
    SendAnimation(SendAnimation),
    SendAudio(SendAudio),
    SendContact(SendContact),
    SendDocument(SendDocument),
    SendLocation(SendLocation),
    SendMessage(SendMessage),
    SendPhoto(SendPhoto),
    SendPoll(SendPoll),
    SendDice(SendDice),
    SendSticker(SendSticker),
    SendVenue(SendVenue),
    SendVideo(SendVideo),
    SendVideoNote(SendVideoNote),
    SendVoice(SendVoice),
}

impl SendCopyMethod {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::ForwardMessage(_) => ForwardMessage::NAME,
            Self::SendAnimation(_) => SendAnimation::NAME,
            Self::SendAudio(_) => SendAudio::NAME,
            Self::SendContact(_) => SendContact::NAME,
            Self::SendDocument(_) => SendDocument::NAME,
            Self::SendLocation(_) => SendLocation::NAME,
            Self::SendMessage(_) => SendMessage::NAME,
            Self::SendPhoto(_) => SendPhoto::NAME,
            Self::SendPoll(_) => SendPoll::NAME,
            Self::SendDice(_) => SendDice::NAME,
            Self::SendSticker(_) => SendSticker::NAME,
            Self::SendVenue(_) => SendVenue::NAME,
            Self::SendVideo(_) => SendVideo::NAME,
            Self::SendVideoNote(_) => SendVideoNote::NAME,
            Self::SendVoice(_) => SendVoice::NAME,
        }
    }

    pub fn payload(&self) -> serde_json::Result<serde_json::Value> {
        match self {
            Self::ForwardMessage(method) => serde_json::to_value(method),
            Self::SendAnimation(method) => serde_json::to_value(method),
            Self::SendAudio(method) => serde_json::to_value(method),
            Self::SendContact(method) => serde_json::to_value(method),
            Self::SendDocument(method) => serde_json::to_value(method),
            Self::SendLocation(method) => serde_json::to_value(method),
            Self::SendMessage(method) => serde_json::to_value(method),
            Self::SendPhoto(method) => serde_json::to_value(method),
            Self::SendPoll(method) => serde_json::to_value(method),
            Self::SendDice(method) => serde_json::to_value(method),
            Self::SendSticker(method) => serde_json::to_value(method),
            Self::SendVenue(method) => serde_json::to_value(method),
            Self::SendVideo(method) => serde_json::to_value(method),
            Self::SendVideoNote(method) => serde_json::to_value(method),
            Self::SendVoice(method) => serde_json::to_value(method),
        }
    }
}

pub trait TelegramMethod: Serialize + CollectFiles + Send + Sync {
    type Response: DeserializeOwned + Send + 'static;
    const NAME: &'static str;
    const FIELDS: &'static [&'static str];
    const DEFAULT_PROPERTIES: &'static [(&'static str, &'static str)] = &[];

    #[doc(hidden)]
    fn files(&self) -> Vec<InputFileUpload> {
        let mut files = Vec::new();
        self.collect_files(&mut files);
        files
    }
}

/// Result returned by Telegram edit/game methods that can produce either the
/// edited message or a boolean success marker for inline messages.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum MessageOrBool {
    Message(Box<crate::types::Message>),
    Bool(bool),
}

impl MessageOrBool {
    pub fn message(&self) -> Option<&crate::types::Message> {
        match self {
            Self::Message(message) => Some(message),
            Self::Bool(_) => None,
        }
    }

    pub fn is_success(&self) -> bool {
        match self {
            Self::Message(_) => true,
            Self::Bool(value) => *value,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct TelegramResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub error_code: Option<u16>,
    pub description: Option<String>,
    pub parameters: Option<crate::error::ResponseParameters>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enums::ParseMode;
    use crate::types::{ChatId, InputFile, InputFileContent, InputMediaPhoto, MediaUnion};

    #[test]
    fn generated_inventory_matches_upstream_methods() {
        assert_eq!(API_METHOD_COUNT, 185);
    }

    #[test]
    fn union_method_response_is_typed() {
        let result: MessageOrBool = serde_json::from_value(serde_json::json!(true)).unwrap();
        assert!(result.is_success());
        assert!(result.message().is_none());
    }

    #[test]
    fn generated_method_uses_telegram_field_names() {
        let method = SendMessage::new(42_i64, "hello")
            .parse_mode(ParseMode::Html)
            .disable_notification(true);
        let json = serde_json::to_value(method).unwrap();
        assert_eq!(json["chat_id"], 42);
        assert_eq!(json["text"], "hello");
        assert_eq!(json["parse_mode"], "HTML");
        assert_eq!(json["disable_notification"], true);
    }

    #[test]
    fn generated_method_discovers_file_uploads() {
        let method = SendPhoto::new(
            ChatId::Id(42),
            InputFile::named_bytes("photo_upload", "avatar.jpg", b"image".to_vec()),
        );
        let files = method.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].attachment_name, "photo_upload");
        assert_eq!(files[0].file_name, "avatar.jpg");
        assert!(matches!(
            &files[0].content,
            InputFileContent::Bytes(data) if data.as_ref() == b"image"
        ));
        assert_eq!(
            serde_json::to_value(method).unwrap()["photo"],
            "attach://photo_upload"
        );
    }

    #[test]
    fn file_references_use_ergonomic_into_conversion() {
        let method = SendPhoto::new(42_i64, "telegram-file-id");
        assert_eq!(
            serde_json::to_value(method).unwrap()["photo"],
            "telegram-file-id"
        );
    }

    #[test]
    fn nested_input_media_uploads_are_discovered() {
        let media = InputMediaPhoto::new(InputFile::named_bytes(
            "album_photo",
            "album.jpg",
            b"album-image".to_vec(),
        ));
        let method = SendMediaGroup::new(42_i64, vec![MediaUnion::from(media)]);
        let files = method.files();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].attachment_name, "album_photo");
        assert_eq!(
            serde_json::to_value(method).unwrap()["media"][0]["media"],
            "attach://album_photo"
        );
    }
}
