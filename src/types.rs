//! Telegram Bot API types generated from the pinned upstream schema.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

mod generated;
pub use generated::*;
mod bound;
pub use bound::BOUND_METHOD_COUNT;

/// Telegram objects that expose a file identifier accepted by `getFile`.
pub trait Downloadable {
    fn file_id(&self) -> &str;
}

macro_rules! impl_downloadable {
    ($($kind:ty),+ $(,)?) => {
        $(impl Downloadable for $kind {
            fn file_id(&self) -> &str {
                &self.file_id
            }
        })+
    };
}

impl_downloadable!(
    Animation,
    Audio,
    Document,
    File,
    LivePhoto,
    PassportFile,
    PhotoSize,
    Sticker,
    Video,
    VideoNote,
    VideoQuality,
    Voice,
);

impl Chat {
    /// Returns the positive chat identifier without Telegram's `-100` prefix.
    /// This is the identifier used by private `t.me/c/...` links.
    pub fn shifted_id(&self) -> i64 {
        let short_id = self.id.to_string().replace("-100", "");
        let shift = -10_i64.pow((short_id.len() + 2) as u32);
        shift - self.id
    }

    /// Returns the title for group-like chats and the user's full name for a
    /// private chat.
    pub fn full_name(&self) -> String {
        if let Some(title) = &self.title {
            return title.clone();
        }
        match (&self.first_name, &self.last_name) {
            (Some(first_name), Some(last_name)) => format!("{first_name} {last_name}"),
            (Some(first_name), None) => first_name.clone(),
            (None, Some(last_name)) => last_name.clone(),
            (None, None) => String::new(),
        }
    }
}

impl Contact {
    pub fn full_name(&self) -> String {
        self.last_name.as_ref().map_or_else(
            || self.first_name.clone(),
            |last_name| format!("{} {last_name}", self.first_name),
        )
    }
}

impl User {
    pub fn full_name(&self) -> String {
        self.last_name.as_ref().map_or_else(
            || self.first_name.clone(),
            |last_name| format!("{} {last_name}", self.first_name),
        )
    }

    pub fn url(&self) -> String {
        crate::utils::link::create_tg_link("user", [("id", self.id)])
    }

    pub fn mention_markdown(&self) -> String {
        self.mention_markdown_as(&self.full_name())
    }

    pub fn mention_markdown_as(&self, name: &str) -> String {
        crate::utils::formatting::markdown_v2_link(name, &self.url())
    }

    pub fn mention_html(&self) -> String {
        self.mention_html_as(&self.full_name())
    }

    pub fn mention_html_as(&self, name: &str) -> String {
        crate::utils::formatting::html_link(name, &self.url())
    }
}

impl Message {
    /// Builds reply coordinates for this message, including ephemeral-message
    /// addressing rules used by aiogram's `reply*` shortcuts.
    pub fn as_reply_parameters(&self) -> ReplyParameters {
        if let Some(ephemeral_message_id) = self.ephemeral_message_id {
            ReplyParameters::new().ephemeral_message_id(ephemeral_message_id)
        } else {
            ReplyParameters::new()
                .message_id(self.message_id)
                .chat_id(self.chat.id)
        }
    }

    /// Renders the message text (or media caption) using Telegram HTML markup.
    pub fn html_text(&self) -> crate::Result<String> {
        let text = self
            .text
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(self.caption.as_deref())
            .unwrap_or_default();
        let entities = self
            .entities
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(self.caption_entities.as_deref())
            .unwrap_or_default();
        crate::utils::formatting::html_text(text, entities)
    }

    /// Renders the message text (or media caption) using Telegram MarkdownV2.
    pub fn markdown_text(&self) -> crate::Result<String> {
        let text = self
            .text
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(self.caption.as_deref())
            .unwrap_or_default();
        let entities = self
            .entities
            .as_deref()
            .filter(|value| !value.is_empty())
            .or(self.caption_entities.as_deref())
            .unwrap_or_default();
        crate::utils::formatting::markdown_text(text, entities)
    }

    /// Aiogram-compatible short alias for [`Self::markdown_text`].
    pub fn md_text(&self) -> crate::Result<String> {
        self.markdown_text()
    }

    /// Returns the public or internal Telegram URL for a channel/supergroup
    /// message. Private and basic group messages do not have such links.
    pub fn get_url(&self) -> Option<String> {
        self.get_url_with_options(false, false)
    }

    pub fn get_url_with_options(
        &self,
        force_private: bool,
        include_thread_id: bool,
    ) -> Option<String> {
        if matches!(self.chat.kind.as_str(), "private" | "group") {
            return None;
        }
        let chat_value = if !force_private {
            self.chat.username.clone()
        } else {
            None
        }
        .unwrap_or_else(|| format!("c/{}", self.chat.shifted_id()));
        let message_value = if include_thread_id
            && self.is_topic_message == Some(true)
            && self.message_thread_id.is_some()
        {
            format!(
                "{}/{}",
                self.message_thread_id.unwrap_or_default(),
                self.message_id
            )
        } else {
            self.message_id.to_string()
        };
        Some(format!("https://t.me/{chat_value}/{message_value}"))
    }

    /// Recreates this message using the corresponding send method and returns
    /// a typed request which, unlike `copyMessage`, yields the sent `Message`.
    pub fn send_copy(
        &self,
        chat_id: impl Into<ChatId>,
    ) -> crate::Result<crate::methods::SendCopyMethod> {
        self.send_copy_with_options(chat_id, crate::methods::SendCopyOptions::default())
    }

    pub fn send_copy_with_options(
        &self,
        chat_id: impl Into<ChatId>,
        options: crate::methods::SendCopyOptions,
    ) -> crate::Result<crate::methods::SendCopyMethod> {
        use crate::methods::{
            ForwardMessage, SendAnimation, SendAudio, SendContact, SendCopyMethod, SendDice,
            SendDocument, SendLocation, SendMessage, SendPhoto, SendPoll, SendSticker, SendVenue,
            SendVideo, SendVideoNote, SendVoice,
        };

        let chat_id = chat_id.into();
        let reply_markup = options
            .reply_markup
            .clone()
            .or_else(|| self.reply_markup.clone().map(Into::into));
        let message_effect_id = options
            .message_effect_id
            .clone()
            .or_else(|| self.effect_id.clone());

        macro_rules! apply_common {
            ($method:expr) => {{
                let mut method = $method;
                method.business_connection_id = options.business_connection_id.clone();
                method.message_thread_id = options.message_thread_id;
                method.disable_notification = options.disable_notification;
                method.reply_to_message_id = options.reply_to_message_id;
                method.reply_parameters = options.reply_parameters.clone();
                method.reply_markup = reply_markup.clone();
                method.allow_sending_without_reply = options.allow_sending_without_reply;
                method.message_effect_id = message_effect_id.clone();
                if options.disable_notification.is_none() {
                    method
                        .extra
                        .insert("disable_notification".to_owned(), serde_json::Value::Null);
                }
                if options.allow_sending_without_reply.is_none() {
                    method.extra.insert(
                        "allow_sending_without_reply".to_owned(),
                        serde_json::Value::Null,
                    );
                }
                method
            }};
        }

        macro_rules! apply_caption {
            ($method:expr) => {{
                let mut method = apply_common!($method);
                method.parse_mode = options.parse_mode.clone();
                if options.parse_mode.is_none() {
                    method
                        .extra
                        .insert("parse_mode".to_owned(), serde_json::Value::Null);
                }
                method
            }};
        }

        if let Some(text) = &self.text {
            let mut method = apply_caption!(SendMessage::new(chat_id.clone(), text.clone()));
            method.entities = self.entities.clone();
            method.link_preview_options = options
                .link_preview_options
                .clone()
                .or_else(|| self.link_preview_options.clone());
            if method.link_preview_options.is_none() {
                method
                    .extra
                    .insert("link_preview_options".to_owned(), serde_json::Value::Null);
            }
            return Ok(SendCopyMethod::SendMessage(method));
        }
        if let Some(audio) = &self.audio {
            let mut method =
                apply_caption!(SendAudio::new(chat_id.clone(), audio.file_id.clone(),));
            method.caption = self.caption.clone();
            method.title = audio.title.clone();
            method.performer = audio.performer.clone();
            method.duration = Some(audio.duration);
            method.caption_entities = self.caption_entities.clone();
            return Ok(SendCopyMethod::SendAudio(method));
        }
        if let Some(animation) = &self.animation {
            let mut method = apply_caption!(SendAnimation::new(
                chat_id.clone(),
                animation.file_id.clone(),
            ));
            method.caption = self.caption.clone();
            method.caption_entities = self.caption_entities.clone();
            return Ok(SendCopyMethod::SendAnimation(method));
        }
        if let Some(document) = &self.document {
            let mut method =
                apply_caption!(SendDocument::new(chat_id.clone(), document.file_id.clone(),));
            method.caption = self.caption.clone();
            method.caption_entities = self.caption_entities.clone();
            return Ok(SendCopyMethod::SendDocument(method));
        }
        if let Some(photo) = self.photo.as_ref().and_then(|sizes| sizes.last()) {
            let mut method = apply_caption!(SendPhoto::new(chat_id.clone(), photo.file_id.clone()));
            method.caption = self.caption.clone();
            method.caption_entities = self.caption_entities.clone();
            return Ok(SendCopyMethod::SendPhoto(method));
        }
        if let Some(sticker) = &self.sticker {
            let method = apply_common!(SendSticker::new(chat_id.clone(), sticker.file_id.clone(),));
            return Ok(SendCopyMethod::SendSticker(method));
        }
        if let Some(video) = &self.video {
            let mut method = apply_caption!(SendVideo::new(chat_id.clone(), video.file_id.clone()));
            method.caption = self.caption.clone();
            method.caption_entities = self.caption_entities.clone();
            return Ok(SendCopyMethod::SendVideo(method));
        }
        if let Some(video_note) = &self.video_note {
            let method = apply_common!(SendVideoNote::new(
                chat_id.clone(),
                video_note.file_id.clone(),
            ));
            return Ok(SendCopyMethod::SendVideoNote(method));
        }
        if let Some(voice) = &self.voice {
            let method = apply_caption!(SendVoice::new(chat_id.clone(), voice.file_id.clone()));
            return Ok(SendCopyMethod::SendVoice(method));
        }
        if let Some(contact) = &self.contact {
            let mut method = apply_common!(SendContact::new(
                chat_id.clone(),
                contact.phone_number.clone(),
                contact.first_name.clone(),
            ));
            method.last_name = contact.last_name.clone();
            method.vcard = contact.vcard.clone();
            return Ok(SendCopyMethod::SendContact(method));
        }
        if let Some(venue) = &self.venue {
            let mut method = apply_common!(SendVenue::new(
                chat_id.clone(),
                venue.location.latitude,
                venue.location.longitude,
                venue.title.clone(),
                venue.address.clone(),
            ));
            method.foursquare_id = venue.foursquare_id.clone();
            method.foursquare_type = venue.foursquare_type.clone();
            return Ok(SendCopyMethod::SendVenue(method));
        }
        if let Some(location) = &self.location {
            let method = apply_common!(SendLocation::new(
                chat_id.clone(),
                location.latitude,
                location.longitude,
            ));
            return Ok(SendCopyMethod::SendLocation(method));
        }
        if let Some(poll) = &self.poll {
            let poll_options = poll
                .options
                .iter()
                .map(|option| {
                    let mut input = InputPollOption::new(option.text.clone());
                    input.text_entities = option.text_entities.clone();
                    input.extra.insert(
                        "voter_count".to_owned(),
                        serde_json::Value::from(option.voter_count),
                    );
                    InputPollOptionUnion::from(input)
                })
                .collect();
            let method = apply_common!(SendPoll::new(
                chat_id.clone(),
                poll.question.clone(),
                poll_options,
            ));
            return Ok(SendCopyMethod::SendPoll(method));
        }
        if self.dice.is_some() {
            return Ok(SendCopyMethod::SendDice(apply_common!(SendDice::new(
                chat_id.clone()
            ))));
        }
        if self.story.is_some() {
            let mut method = ForwardMessage::new(chat_id, self.chat.id, self.message_id);
            method.message_thread_id = options.message_thread_id;
            method.disable_notification = options.disable_notification;
            method.message_effect_id = message_effect_id;
            return Ok(SendCopyMethod::ForwardMessage(method));
        }

        Err(crate::Error::Utility(
            "This type of message can't be copied.".to_owned(),
        ))
    }
}

impl MessageEntity {
    /// Extracts this entity's substring using Telegram's UTF-16 offsets.
    pub fn extract_from(&self, text: &str) -> crate::Result<String> {
        crate::utils::formatting::extract_entity_text(text, self)
    }
}

impl InaccessibleMessage {
    pub fn as_reply_parameters(&self) -> ReplyParameters {
        ReplyParameters::new()
            .message_id(self.message_id)
            .chat_id(self.chat.id)
    }
}

/// Internal upload descriptor used by typed methods.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFileUpload {
    pub attachment_name: String,
    pub file_name: String,
    pub content: InputFileContent,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputFileContent {
    Bytes(Arc<[u8]>),
    Path(PathBuf),
    Url {
        url: String,
        headers: BTreeMap<String, String>,
        timeout: Duration,
    },
}

/// Recursively discovers in-memory files in a Telegram method payload.
#[doc(hidden)]
pub trait CollectFiles {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>);
}

macro_rules! empty_collect_files {
    ($($kind:ty),* $(,)?) => {
        $(impl CollectFiles for $kind {
            fn collect_files(&self, _output: &mut Vec<InputFileUpload>) {}
        })*
    };
}

empty_collect_files!(bool, i64, f64, String, Value, ChatId);

impl<T: CollectFiles> CollectFiles for Option<T> {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {
        if let Some(value) = self {
            value.collect_files(output);
        }
    }
}

impl<T: CollectFiles> CollectFiles for Vec<T> {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {
        for value in self {
            value.collect_files(output);
        }
    }
}

impl<T: CollectFiles> CollectFiles for Box<T> {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {
        self.as_ref().collect_files(output);
    }
}

impl<K, T: CollectFiles> CollectFiles for BTreeMap<K, T> {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {
        for value in self.values() {
            value.collect_files(output);
        }
    }
}

/// Integer chat id or public `@username` accepted by Telegram methods.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatId {
    Id(i64),
    Username(String),
}

impl From<i64> for ChatId {
    fn from(value: i64) -> Self {
        Self::Id(value)
    }
}

impl From<String> for ChatId {
    fn from(value: String) -> Self {
        Self::Username(value)
    }
}

impl From<&str> for ChatId {
    fn from(value: &str) -> Self {
        Self::Username(value.to_owned())
    }
}

impl From<String> for InputFile {
    fn from(value: String) -> Self {
        Self::Reference(value)
    }
}

impl From<&str> for InputFile {
    fn from(value: &str) -> Self {
        Self::Reference(value.to_owned())
    }
}

/// A Telegram file reference or a new in-memory upload.
#[derive(Clone, PartialEq, Eq)]
pub enum InputFile {
    Reference(String),
    Memory {
        attachment_name: String,
        file_name: String,
        data: Arc<[u8]>,
    },
    Path {
        attachment_name: String,
        file_name: String,
        path: PathBuf,
    },
    Url {
        attachment_name: String,
        file_name: String,
        url: String,
        headers: BTreeMap<String, String>,
        timeout: Duration,
    },
}

impl InputFile {
    pub fn reference(value: impl Into<String>) -> Self {
        Self::Reference(value.into())
    }

    pub fn bytes(file_name: impl Into<String>, data: impl Into<Vec<u8>>) -> Self {
        let file_name = file_name.into();
        Self::Memory {
            attachment_name: file_name.clone(),
            file_name,
            data: data.into().into(),
        }
    }

    /// Creates an upload with a distinct multipart attachment id and file name.
    pub fn named_bytes(
        attachment_name: impl Into<String>,
        file_name: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        Self::Memory {
            attachment_name: attachment_name.into(),
            file_name: file_name.into(),
            data: data.into().into(),
        }
    }

    /// Reads a local file only when the Bot API request is executed.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_owned();
        Self::Path {
            attachment_name: file_name.clone(),
            file_name,
            path,
        }
    }

    pub fn named_path(
        attachment_name: impl Into<String>,
        file_name: impl Into<String>,
        path: impl AsRef<Path>,
    ) -> Self {
        Self::Path {
            attachment_name: attachment_name.into(),
            file_name: file_name.into(),
            path: path.as_ref().to_owned(),
        }
    }

    /// Downloads a file through the bot's configured HTTP client when the
    /// multipart request is executed.
    pub fn url(file_name: impl Into<String>, url: impl Into<String>) -> Self {
        let file_name = file_name.into();
        Self::Url {
            attachment_name: file_name.clone(),
            file_name,
            url: url.into(),
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn named_url(
        attachment_name: impl Into<String>,
        file_name: impl Into<String>,
        url: impl Into<String>,
    ) -> Self {
        Self::Url {
            attachment_name: attachment_name.into(),
            file_name: file_name.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            timeout: Duration::from_secs(30),
        }
    }

    pub fn headers(mut self, value: BTreeMap<String, String>) -> Self {
        if let Self::Url { headers, .. } = &mut self {
            *headers = value;
        }
        self
    }

    pub fn timeout(mut self, value: Duration) -> Self {
        if let Self::Url { timeout, .. } = &mut self {
            *timeout = value;
        }
        self
    }

    pub fn attachment_name(&self) -> Option<&str> {
        match self {
            Self::Reference(_) => None,
            Self::Memory {
                attachment_name, ..
            }
            | Self::Path {
                attachment_name, ..
            }
            | Self::Url {
                attachment_name, ..
            } => Some(attachment_name),
        }
    }
}

impl fmt::Debug for InputFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reference(value) => formatter.debug_tuple("Reference").field(value).finish(),
            Self::Memory {
                attachment_name,
                file_name,
                data,
            } => formatter
                .debug_struct("Memory")
                .field("attachment_name", attachment_name)
                .field("file_name", file_name)
                .field("bytes", &data.len())
                .finish(),
            Self::Path {
                attachment_name,
                file_name,
                path,
            } => formatter
                .debug_struct("Path")
                .field("attachment_name", attachment_name)
                .field("file_name", file_name)
                .field("path", path)
                .finish(),
            Self::Url {
                attachment_name,
                file_name,
                url,
                headers,
                timeout,
            } => formatter
                .debug_struct("Url")
                .field("attachment_name", attachment_name)
                .field("file_name", file_name)
                .field("url", url)
                .field("headers", headers)
                .field("timeout", timeout)
                .finish(),
        }
    }
}

impl Serialize for InputFile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Reference(value) => serializer.serialize_str(value),
            Self::Memory {
                attachment_name, ..
            }
            | Self::Path {
                attachment_name, ..
            }
            | Self::Url {
                attachment_name, ..
            } => serializer.serialize_str(&format!("attach://{attachment_name}")),
        }
    }
}

impl<'de> Deserialize<'de> for InputFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::Reference)
    }
}

impl CollectFiles for InputFile {
    fn collect_files(&self, output: &mut Vec<InputFileUpload>) {
        if let Self::Memory {
            attachment_name,
            file_name,
            data,
        } = self
        {
            output.push(InputFileUpload {
                attachment_name: attachment_name.clone(),
                file_name: file_name.clone(),
                content: InputFileContent::Bytes(data.clone()),
            });
        } else if let Self::Path {
            attachment_name,
            file_name,
            path,
        } = self
        {
            output.push(InputFileUpload {
                attachment_name: attachment_name.clone(),
                file_name: file_name.clone(),
                content: InputFileContent::Path(path.clone()),
            });
        } else if let Self::Url {
            attachment_name,
            file_name,
            url,
            headers,
            timeout,
        } = self
        {
            output.push(InputFileUpload {
                attachment_name: attachment_name.clone(),
                file_name: file_name.clone(),
                content: InputFileContent::Url {
                    url: url.clone(),
                    headers: headers.clone(),
                    timeout: *timeout,
                },
            });
        }
    }
}

impl Message {
    /// Returns text for text messages or the caption for media messages.
    pub fn content(&self) -> Option<&str> {
        self.text.as_deref().or(self.caption.as_deref())
    }

    /// Returns the first concrete content kind using aiogram's field order.
    pub fn content_type(&self) -> crate::enums::ContentType {
        macro_rules! detect {
            ($($field:ident => $variant:ident),+ $(,)?) => {
                $(if self.$field.is_some() {
                    return crate::enums::ContentType::$variant;
                })+
            };
        }
        detect!(
            text => Text,
            audio => Audio,
            animation => Animation,
            document => Document,
            game => Game,
            photo => Photo,
            sticker => Sticker,
            video => Video,
            video_note => VideoNote,
            voice => Voice,
            checklist => Checklist,
            contact => Contact,
            venue => Venue,
            location => Location,
            new_chat_members => NewChatMembers,
            left_chat_member => LeftChatMember,
            chat_owner_left => ChatOwnerLeft,
            chat_owner_changed => ChatOwnerChanged,
            invoice => Invoice,
            successful_payment => SuccessfulPayment,
            users_shared => UsersShared,
            connected_website => ConnectedWebsite,
            migrate_from_chat_id => MigrateFromChatId,
            migrate_to_chat_id => MigrateToChatId,
            pinned_message => PinnedMessage,
            new_chat_title => NewChatTitle,
            new_chat_photo => NewChatPhoto,
            delete_chat_photo => DeleteChatPhoto,
            group_chat_created => GroupChatCreated,
            supergroup_chat_created => SupergroupChatCreated,
            channel_chat_created => ChannelChatCreated,
            paid_media => PaidMedia,
            passport_data => PassportData,
            proximity_alert_triggered => ProximityAlertTriggered,
            poll => Poll,
            dice => Dice,
            message_auto_delete_timer_changed => MessageAutoDeleteTimerChanged,
            forum_topic_created => ForumTopicCreated,
            forum_topic_edited => ForumTopicEdited,
            forum_topic_closed => ForumTopicClosed,
            forum_topic_reopened => ForumTopicReopened,
            general_forum_topic_hidden => GeneralForumTopicHidden,
            general_forum_topic_unhidden => GeneralForumTopicUnhidden,
            giveaway_created => GiveawayCreated,
            giveaway => Giveaway,
            giveaway_completed => GiveawayCompleted,
            giveaway_winners => GiveawayWinners,
            video_chat_scheduled => VideoChatScheduled,
            video_chat_started => VideoChatStarted,
            video_chat_ended => VideoChatEnded,
            video_chat_participants_invited => VideoChatParticipantsInvited,
            web_app_data => WebAppData,
            user_shared => UserShared,
            chat_shared => ChatShared,
            story => Story,
            write_access_allowed => WriteAccessAllowed,
            chat_background_set => ChatBackgroundSet,
            boost_added => BoostAdded,
            checklist_tasks_done => ChecklistTasksDone,
            checklist_tasks_added => ChecklistTasksAdded,
            direct_message_price_changed => DirectMessagePriceChanged,
            refunded_payment => RefundedPayment,
            gift => Gift,
            unique_gift => UniqueGift,
            gift_upgrade_sent => GiftUpgradeSent,
            paid_message_price_changed => PaidMessagePriceChanged,
            suggested_post_approved => SuggestedPostApproved,
            suggested_post_approval_failed => SuggestedPostApprovalFailed,
            suggested_post_declined => SuggestedPostDeclined,
            suggested_post_paid => SuggestedPostPaid,
            suggested_post_refunded => SuggestedPostRefunded,
            managed_bot_created => ManagedBotCreated,
            poll_option_added => PollOptionAdded,
            poll_option_deleted => PollOptionDeleted,
            live_photo => LivePhoto,
            rich_message => RichMessage,
            community_chat_added => CommunityChatAdded,
            community_chat_removed => CommunityChatRemoved,
        );
        crate::enums::ContentType::Unknown
    }
}

/// Type-safe counterpart of aiogram's dynamic `Update.event` property.
#[derive(Debug, Clone, Copy)]
pub enum UpdateEventRef<'a> {
    Message(&'a Message),
    EditedMessage(&'a Message),
    ChannelPost(&'a Message),
    EditedChannelPost(&'a Message),
    InlineQuery(&'a InlineQuery),
    ChosenInlineResult(&'a ChosenInlineResult),
    CallbackQuery(&'a CallbackQuery),
    ShippingQuery(&'a ShippingQuery),
    PreCheckoutQuery(&'a PreCheckoutQuery),
    Poll(&'a Poll),
    PollAnswer(&'a PollAnswer),
    MyChatMember(&'a ChatMemberUpdated),
    ChatMember(&'a ChatMemberUpdated),
    ChatJoinRequest(&'a ChatJoinRequest),
    MessageReaction(&'a MessageReactionUpdated),
    MessageReactionCount(&'a MessageReactionCountUpdated),
    ChatBoost(&'a ChatBoostUpdated),
    RemovedChatBoost(&'a ChatBoostRemoved),
    DeletedBusinessMessages(&'a BusinessMessagesDeleted),
    BusinessConnection(&'a BusinessConnection),
    EditedBusinessMessage(&'a Message),
    BusinessMessage(&'a Message),
    PurchasedPaidMedia(&'a PaidMediaPurchased),
    GuestMessage(&'a Message),
    ManagedBot(&'a ManagedBotUpdated),
    Subscription(&'a BotSubscriptionUpdated),
}

impl<'a> UpdateEventRef<'a> {
    pub const fn event_type(self) -> &'static str {
        match self {
            Self::Message(_) => "message",
            Self::EditedMessage(_) => "edited_message",
            Self::ChannelPost(_) => "channel_post",
            Self::EditedChannelPost(_) => "edited_channel_post",
            Self::InlineQuery(_) => "inline_query",
            Self::ChosenInlineResult(_) => "chosen_inline_result",
            Self::CallbackQuery(_) => "callback_query",
            Self::ShippingQuery(_) => "shipping_query",
            Self::PreCheckoutQuery(_) => "pre_checkout_query",
            Self::Poll(_) => "poll",
            Self::PollAnswer(_) => "poll_answer",
            Self::MyChatMember(_) => "my_chat_member",
            Self::ChatMember(_) => "chat_member",
            Self::ChatJoinRequest(_) => "chat_join_request",
            Self::MessageReaction(_) => "message_reaction",
            Self::MessageReactionCount(_) => "message_reaction_count",
            Self::ChatBoost(_) => "chat_boost",
            Self::RemovedChatBoost(_) => "removed_chat_boost",
            Self::DeletedBusinessMessages(_) => "deleted_business_messages",
            Self::BusinessConnection(_) => "business_connection",
            Self::EditedBusinessMessage(_) => "edited_business_message",
            Self::BusinessMessage(_) => "business_message",
            Self::PurchasedPaidMedia(_) => "purchased_paid_media",
            Self::GuestMessage(_) => "guest_message",
            Self::ManagedBot(_) => "managed_bot",
            Self::Subscription(_) => "subscription",
        }
    }

    pub const fn as_message(self) -> Option<&'a Message> {
        match self {
            Self::Message(value)
            | Self::EditedMessage(value)
            | Self::ChannelPost(value)
            | Self::EditedChannelPost(value)
            | Self::EditedBusinessMessage(value)
            | Self::BusinessMessage(value)
            | Self::GuestMessage(value) => Some(value),
            _ => None,
        }
    }
}

impl Update {
    /// Returns the first known update payload using aiogram's precedence.
    pub fn event(&self) -> Option<UpdateEventRef<'_>> {
        if let Some(value) = self.message.as_deref() {
            Some(UpdateEventRef::Message(value))
        } else if let Some(value) = self.edited_message.as_deref() {
            Some(UpdateEventRef::EditedMessage(value))
        } else if let Some(value) = self.channel_post.as_deref() {
            Some(UpdateEventRef::ChannelPost(value))
        } else if let Some(value) = self.edited_channel_post.as_deref() {
            Some(UpdateEventRef::EditedChannelPost(value))
        } else if let Some(value) = self.inline_query.as_ref() {
            Some(UpdateEventRef::InlineQuery(value))
        } else if let Some(value) = self.chosen_inline_result.as_ref() {
            Some(UpdateEventRef::ChosenInlineResult(value))
        } else if let Some(value) = self.callback_query.as_ref() {
            Some(UpdateEventRef::CallbackQuery(value))
        } else if let Some(value) = self.shipping_query.as_ref() {
            Some(UpdateEventRef::ShippingQuery(value))
        } else if let Some(value) = self.pre_checkout_query.as_ref() {
            Some(UpdateEventRef::PreCheckoutQuery(value))
        } else if let Some(value) = self.poll.as_ref() {
            Some(UpdateEventRef::Poll(value))
        } else if let Some(value) = self.poll_answer.as_ref() {
            Some(UpdateEventRef::PollAnswer(value))
        } else if let Some(value) = self.my_chat_member.as_ref() {
            Some(UpdateEventRef::MyChatMember(value))
        } else if let Some(value) = self.chat_member.as_ref() {
            Some(UpdateEventRef::ChatMember(value))
        } else if let Some(value) = self.chat_join_request.as_ref() {
            Some(UpdateEventRef::ChatJoinRequest(value))
        } else if let Some(value) = self.message_reaction.as_ref() {
            Some(UpdateEventRef::MessageReaction(value))
        } else if let Some(value) = self.message_reaction_count.as_ref() {
            Some(UpdateEventRef::MessageReactionCount(value))
        } else if let Some(value) = self.chat_boost.as_ref() {
            Some(UpdateEventRef::ChatBoost(value))
        } else if let Some(value) = self.removed_chat_boost.as_ref() {
            Some(UpdateEventRef::RemovedChatBoost(value))
        } else if let Some(value) = self.deleted_business_messages.as_ref() {
            Some(UpdateEventRef::DeletedBusinessMessages(value))
        } else if let Some(value) = self.business_connection.as_ref() {
            Some(UpdateEventRef::BusinessConnection(value))
        } else if let Some(value) = self.edited_business_message.as_deref() {
            Some(UpdateEventRef::EditedBusinessMessage(value))
        } else if let Some(value) = self.business_message.as_deref() {
            Some(UpdateEventRef::BusinessMessage(value))
        } else if let Some(value) = self.purchased_paid_media.as_ref() {
            Some(UpdateEventRef::PurchasedPaidMedia(value))
        } else if let Some(value) = self.guest_message.as_deref() {
            Some(UpdateEventRef::GuestMessage(value))
        } else if let Some(value) = self.managed_bot.as_ref() {
            Some(UpdateEventRef::ManagedBot(value))
        } else {
            self.subscription.as_ref().map(UpdateEventRef::Subscription)
        }
    }

    /// Returns the Telegram update field that contains the current event.
    pub fn event_type(&self) -> Option<&'static str> {
        self.event().map(UpdateEventRef::event_type)
    }

    /// Returns the message-like payload for all message update variants.
    pub fn message_event(&self) -> Option<&Message> {
        self.message
            .as_deref()
            .or(self.edited_message.as_deref())
            .or(self.channel_post.as_deref())
            .or(self.edited_channel_post.as_deref())
            .or(self.business_message.as_deref())
            .or(self.edited_business_message.as_deref())
            .or(self.guest_message.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_inventory_matches_upstream_entities() {
        assert_eq!(API_ENTITY_COUNT, 390);
        assert_eq!(API_UNION_COUNT, 35);
        assert_eq!(BOUND_METHOD_COUNT, 187);
    }

    #[test]
    fn generated_union_preserves_concrete_chat_member() {
        let owner = ChatMemberOwner::new(User::new(1, false, "Ada".to_owned()), false);
        let json = serde_json::to_value(ChatMemberUnion::from(owner)).unwrap();
        let decoded: ChatMemberUnion = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded, ChatMemberUnion::ChatMemberOwner(_)));
    }

    #[test]
    fn downloadable_trait_covers_media_and_file_descriptors() {
        let photo = PhotoSize::new("photo-id", "unique", 10, 20);
        let file = File::new("file-id", "unique");
        assert_eq!(Downloadable::file_id(&photo), "photo-id");
        assert_eq!(Downloadable::file_id(&file), "file-id");
    }

    #[test]
    fn hand_written_name_mention_and_message_url_helpers_match_aiogram() {
        let user = User::new(42, false, "Ada").last_name("Lovelace");
        assert_eq!(user.full_name(), "Ada Lovelace");
        assert_eq!(user.url(), "tg://user?id=42");
        assert_eq!(
            user.mention_html(),
            "<a href=\"tg://user?id=42\">Ada Lovelace</a>"
        );
        assert_eq!(user.mention_markdown_as("Ada"), "[Ada](tg://user?id=42)");

        let contact = Contact::new("+100", "Grace").last_name("Hopper");
        assert_eq!(contact.full_name(), "Grace Hopper");

        let chat: Chat = serde_json::from_value(serde_json::json!({
            "id": -1001234567890_i64,
            "type": "supergroup",
            "title": "Rustaceans",
            "username": "rustaceans"
        }))
        .unwrap();
        assert_eq!(chat.shifted_id(), 1_234_567_890);
        assert_eq!(chat.full_name(), "Rustaceans");

        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 77,
            "date": 1,
            "chat": chat,
            "message_thread_id": 12,
            "is_topic_message": true
        }))
        .unwrap();
        assert_eq!(
            message.get_url().as_deref(),
            Some("https://t.me/rustaceans/77")
        );
        assert_eq!(
            message.get_url_with_options(true, true).as_deref(),
            Some("https://t.me/c/1234567890/12/77")
        );
    }

    #[test]
    fn message_content_type_uses_exact_upstream_precedence() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 1,
            "date": 2,
            "chat": {"id": 3, "type": "private"},
            "animation": {
                "file_id": "animation",
                "file_unique_id": "animation-unique",
                "width": 1,
                "height": 1,
                "duration": 1
            },
            "audio": {
                "file_id": "audio",
                "file_unique_id": "audio-unique",
                "duration": 1
            }
        }))
        .unwrap();
        assert_eq!(message.content_type(), crate::enums::ContentType::Audio);
    }

    #[test]
    fn update_event_is_typed_and_uses_upstream_precedence() {
        let update: Update = serde_json::from_value(serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "date": 1,
                "chat": {"id": 1, "type": "private"},
                "text": "first"
            },
            "business_connection": {
                "id": "connection",
                "user": {"id": 1, "is_bot": false, "first_name": "Ada"},
                "user_chat_id": 1,
                "date": 1,
                "is_enabled": true
            }
        }))
        .unwrap();
        assert_eq!(update.event_type(), Some("message"));
        assert!(matches!(update.event(), Some(UpdateEventRef::Message(_))));
        assert_eq!(
            update
                .event()
                .unwrap()
                .as_message()
                .unwrap()
                .text
                .as_deref(),
            Some("first")
        );
    }

    #[test]
    fn deserializes_full_generated_message_update_and_keeps_new_fields() {
        let update: Update = serde_json::from_value(serde_json::json!({
            "update_id": 42,
            "message": {
                "message_id": 7,
                "date": 1_700_000_000,
                "chat": {"id": 10, "type": "private"},
                "from": {"id": 11, "is_bot": false, "first_name": "Ada"},
                "text": "/start",
                "future_message_field": {"enabled": true}
            },
            "future_update": {"id": "x"}
        }))
        .unwrap();

        assert_eq!(update.event_type(), Some("message"));
        assert_eq!(
            update.message_event().and_then(Message::content),
            Some("/start")
        );
        assert_eq!(
            update.message_event().map(Message::content_type),
            Some(crate::enums::ContentType::Text)
        );
        assert!(
            update
                .message
                .unwrap()
                .extra
                .contains_key("future_message_field")
        );
        assert!(update.extra.contains_key("future_update"));
    }

    #[test]
    fn generated_bound_methods_fill_message_and_query_coordinates() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 7,
            "date": 1_700_000_000,
            "chat": {"id": 10, "type": "private"},
            "from": {"id": 11, "is_bot": false, "first_name": "Ada"},
            "business_connection_id": "business-1",
            "text": "incoming"
        }))
        .unwrap();

        let answer = serde_json::to_value(message.answer("hello").unwrap()).unwrap();
        assert_eq!(answer["chat_id"], 10);
        assert_eq!(answer["text"], "hello");
        assert_eq!(answer["business_connection_id"], "business-1");

        let reply = serde_json::to_value(message.reply("hello").unwrap()).unwrap();
        assert_eq!(reply["reply_parameters"]["message_id"], 7);
        assert_eq!(reply["reply_parameters"]["chat_id"], 10);

        let invoice = message
            .answer_invoice(
                "Product",
                "Description",
                "payload",
                "USD",
                vec![LabeledPrice::new("Price", 100)],
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(invoice).unwrap()["business_connection_id"],
            "business-1"
        );
        assert!(message.edit_ephemeral_text("new text").is_err());

        let query = CallbackQuery::new("callback-1", User::new(11, false, "Ada"), "instance");
        assert_eq!(
            serde_json::to_value(query.answer().unwrap()).unwrap()["callback_query_id"],
            "callback-1"
        );
    }

    #[test]
    fn send_copy_selects_every_aiogram_supported_method() {
        let cases = [
            ("text", serde_json::json!("hello"), "sendMessage"),
            (
                "audio",
                serde_json::json!({"file_id":"audio-id","file_unique_id":"audio-u","duration":3}),
                "sendAudio",
            ),
            (
                "animation",
                serde_json::json!({"file_id":"animation-id","file_unique_id":"animation-u","width":10,"height":10,"duration":3}),
                "sendAnimation",
            ),
            (
                "document",
                serde_json::json!({"file_id":"document-id","file_unique_id":"document-u"}),
                "sendDocument",
            ),
            (
                "photo",
                serde_json::json!([{"file_id":"photo-id","file_unique_id":"photo-u","width":10,"height":10}]),
                "sendPhoto",
            ),
            (
                "sticker",
                serde_json::json!({"file_id":"sticker-id","file_unique_id":"sticker-u","type":"regular","width":10,"height":10,"is_animated":false,"is_video":false}),
                "sendSticker",
            ),
            (
                "video",
                serde_json::json!({"file_id":"video-id","file_unique_id":"video-u","width":10,"height":10,"duration":3}),
                "sendVideo",
            ),
            (
                "video_note",
                serde_json::json!({"file_id":"note-id","file_unique_id":"note-u","length":10,"duration":3}),
                "sendVideoNote",
            ),
            (
                "voice",
                serde_json::json!({"file_id":"voice-id","file_unique_id":"voice-u","duration":3}),
                "sendVoice",
            ),
            (
                "contact",
                serde_json::json!({"phone_number":"+10000000000","first_name":"Ada"}),
                "sendContact",
            ),
            (
                "venue",
                serde_json::json!({"location":{"latitude":1.25,"longitude":2.5},"title":"Office","address":"Main street"}),
                "sendVenue",
            ),
            (
                "location",
                serde_json::json!({"latitude":1.25,"longitude":2.5}),
                "sendLocation",
            ),
            (
                "poll",
                serde_json::json!({
                    "id":"poll-id","question":"Choose","options":[{"persistent_id":"a","text":"One","voter_count":2}],
                    "total_voter_count":2,"is_closed":false,"is_anonymous":true,"type":"regular",
                    "allows_multiple_answers":false,"allows_revoting":true,"members_only":false
                }),
                "sendPoll",
            ),
            (
                "dice",
                serde_json::json!({"emoji":"🎲","value":4}),
                "sendDice",
            ),
            (
                "story",
                serde_json::json!({"chat":{"id":10,"type":"private"},"id":9}),
                "forwardMessage",
            ),
        ];

        for (field, content, expected_method) in cases {
            let mut value = serde_json::json!({
                "message_id": 7,
                "date": 1_700_000_000,
                "chat": {"id": 10, "type": "private"}
            });
            value
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), content);
            let message: Message = serde_json::from_value(value).unwrap();
            let method = message.send_copy(42_i64).unwrap();
            assert_eq!(method.method_name(), expected_method, "field {field}");
            assert_eq!(method.payload().unwrap()["chat_id"], 42, "field {field}");
        }
    }

    #[test]
    fn send_copy_preserves_entities_effects_and_explicit_overrides() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 7,
            "date": 1_700_000_000,
            "chat": {"id": 10, "type": "private"},
            "text": "bold",
            "entities": [{"type":"bold","offset":0,"length":4}],
            "effect_id": "source-effect",
            "link_preview_options": {"prefer_small_media":true}
        }))
        .unwrap();
        let options = crate::methods::SendCopyOptions::default()
            .parse_mode("MarkdownV2")
            .message_effect_id("override-effect")
            .link_preview_options(LinkPreviewOptions::new().is_disabled(true));
        let method = message.send_copy_with_options(42_i64, options).unwrap();
        let payload = method.payload().unwrap();
        assert_eq!(payload["entities"][0]["type"], "bold");
        assert_eq!(payload["parse_mode"], "MarkdownV2");
        assert_eq!(payload["message_effect_id"], "override-effect");
        assert_eq!(payload["link_preview_options"]["is_disabled"], true);

        let unsupported = Message::new(1, 1, Box::new(Chat::new(10, "private")));
        assert!(unsupported.send_copy(42_i64).is_err());
    }

    #[test]
    fn message_renders_and_extracts_utf16_entities() {
        let message: Message = serde_json::from_value(serde_json::json!({
            "message_id": 7,
            "date": 1_700_000_000,
            "chat": {"id": 10, "type": "private"},
            "text": "🙂 bold",
            "entities": [{"type":"bold","offset":3,"length":4}]
        }))
        .unwrap();
        assert_eq!(message.html_text().unwrap(), "🙂 <b>bold</b>");
        assert_eq!(message.markdown_text().unwrap(), "🙂 *bold*");
        assert_eq!(
            message.entities.as_ref().unwrap()[0]
                .extract_from(message.text.as_deref().unwrap())
                .unwrap(),
            "bold"
        );
    }
}
