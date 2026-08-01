use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("BOT_TOKEN").expect("set BOT_TOKEN before running the example");
    let bot = Bot::new(token)?;

    let mut router = Router::named("send-copy");
    router.message(filters::any(), |context| async move {
        let Some(message) = context.message() else {
            return Ok(());
        };
        let method = message.send_copy(message.chat.id)?;
        context.bot.execute_send_copy(&method).await?;
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher
        .include_router(router)
        .allowed_updates(["message"])
        .start_polling(bot)
        .await
}
