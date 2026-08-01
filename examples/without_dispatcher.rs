use aiogram::client::DefaultBotProperties;
use aiogram::{Bot, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let token = arguments
        .next()
        .or_else(|| std::env::var("BOT_TOKEN").ok())
        .expect("pass token as the first argument or set BOT_TOKEN");
    let chat_id = arguments
        .next()
        .expect("pass chat id as the second argument")
        .parse::<i64>()
        .expect("chat id must be an integer");
    let message = arguments
        .next()
        .unwrap_or_else(|| "Hello, World!".to_owned());

    let bot = Bot::builder(token)
        .defaults(DefaultBotProperties::default().parse_mode("HTML"))
        .build()?;
    bot.send_message(chat_id, message).await?;
    Ok(())
}
