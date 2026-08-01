use std::time::Duration;

use aiogram::utils::chat_action::{ChatActionConfig, ChatActionMiddleware};
use aiogram::{Bot, Dispatcher, HandlerFlags, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("flags");
    router.middleware(ChatActionMiddleware::new());

    router.message_with_flags(
        filters::command("work"),
        HandlerFlags::new().with(
            "chat_action",
            ChatActionConfig::new()
                .action("upload_document")
                .initial_sleep(Duration::from_millis(300)),
        ),
        |context| async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            context.answer("Work completed.").await?;
            Ok(())
        },
    );

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
