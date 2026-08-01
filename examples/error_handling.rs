use aiogram::{Bot, Dispatcher, Error, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("errors");

    router.message(filters::command("fail"), |_| async {
        Err(Error::Handler("intentional example failure".to_owned()))
    });
    router.error(filters::any(), |context| async move {
        tracing::error!(error = %context.error().expect("error observer"), "handler failed");
        if context.message().is_some() {
            context.answer("The error was handled safely.").await?;
        }
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
