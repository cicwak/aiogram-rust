#[cfg(feature = "webhook-axum")]
use std::sync::Arc;

#[cfg(feature = "webhook-axum")]
use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[cfg(feature = "webhook-axum")]
#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::new();
    router.message(filters::command("start"), |context| async move {
        context.answer("Webhook is running").await?;
        Ok(())
    });
    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router);
    let app = aiogram::webhook::axum_integration::router(
        "/telegram/webhook",
        Arc::new(dispatcher),
        bot,
        std::env::var("WEBHOOK_SECRET").ok(),
    );
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
    axum::serve(listener, app)
        .await
        .map_err(|error| aiogram::Error::Handler(error.to_string()))
}

#[cfg(not(feature = "webhook-axum"))]
fn main() {
    eprintln!("enable --features webhook-axum to run this example");
}
