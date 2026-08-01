use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let token = std::env::var("BOT_TOKEN").expect("set BOT_TOKEN before running the example");
    let bot = Bot::new(token)?;
    tracing::info!(user = ?bot.get_me().await?, "bot authorized");

    let mut router = Router::named("echo");
    router.message(filters::command("start"), |context| async move {
        context
            .answer("Send me a text message and I will echo it.")
            .await?;
        Ok(())
    });
    router.message(filters::any(), |context| async move {
        if let Some(text) = context.message().and_then(|message| message.text.clone()) {
            context.answer(text).await?;
        }
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher
        .include_router(router)
        .allowed_updates(["message"])
        .start_polling(bot)
        .await
}
