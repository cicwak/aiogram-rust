use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("bound-methods");

    router.message(filters::command("start"), |context| async move {
        let message = context.message().expect("message observer");
        let request = message
            .answer("Bound methods fill chat, topic and business coordinates")?
            .disable_notification(true);
        context.bot.execute(&request).await?;
        Ok(())
    });

    router.callback_query(filters::any(), |context| async move {
        let query = context.callback_query().expect("callback observer");
        context
            .bot
            .execute(&query.answer()?.text("Handled from a bound method"))
            .await?;
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
