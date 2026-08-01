//! Constants exposed by Python aiogram as enums.
//!
//! Telegram object fields remain strings, matching aiogram and preserving
//! forward compatibility when Telegram adds values between crate releases.

mod generated;
pub use generated::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_inventory_and_string_conversions_match_upstream() {
        assert_eq!(API_ENUM_COUNT, 38);
        assert_eq!(ChatAction::UploadPhoto.as_str(), "upload_photo");
        assert_eq!(
            "MarkdownV2".parse::<ParseMode>().unwrap(),
            ParseMode::MarkdownV2
        );
        assert_eq!(String::from(DiceEmoji::Dice), "🎲");
        assert!("future-mode".parse::<ParseMode>().is_err());
    }

    #[test]
    fn integer_enum_uses_telegram_numeric_representation() {
        let encoded = serde_json::to_string(&TopicIconColor::Blue).unwrap();
        assert_eq!(encoded, "7322096");
        assert_eq!(
            serde_json::from_str::<TopicIconColor>(&encoded).unwrap(),
            TopicIconColor::Blue
        );
        assert!(serde_json::from_str::<TopicIconColor>("1").is_err());
    }
}
