#[cfg(feature = "webhook-axum")]
use std::sync::Arc;

#[cfg(feature = "webhook-axum")]
use aiogram::webhook::TokenBotRegistry;
#[cfg(feature = "webhook-axum")]
use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[cfg(feature = "webhook-axum")]
#[tokio::main]
async fn main() -> Result<()> {
    let main_bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;

    let mut main_routes = Router::named("main-bot");
    main_routes.message(filters::command("start"), |context| async move {
        context.answer("Main bot webhook is running").await?;
        Ok(())
    });
    let mut main_dispatcher = Dispatcher::new();
    main_dispatcher.include_router(main_routes);

    let mut other_routes = Router::named("managed-bots");
    other_routes.message(filters::any(), |context| async move {
        context.answer("Hello from a token-routed bot").await?;
        Ok(())
    });
    let mut other_dispatcher = Dispatcher::new();
    other_dispatcher.include_router(other_routes);

    let app = aiogram::webhook::axum_integration::router(
        "/webhook/main",
        Arc::new(main_dispatcher),
        main_bot,
        std::env::var("WEBHOOK_SECRET").ok(),
    )
    .merge(aiogram::webhook::axum_integration::token_router(
        "/webhook/bot/{bot_token}",
        Arc::new(other_dispatcher),
        TokenBotRegistry::new(),
        true,
    )?);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    axum::serve(listener, app)
        .await
        .map_err(|error| aiogram::Error::Handler(error.to_string()))
}

#[cfg(not(feature = "webhook-axum"))]
fn main() {
    eprintln!("enable --features webhook-axum to run this example");
}
