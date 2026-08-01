mod handlers;

use aiogram::{Bot, Dispatcher, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut dispatcher = Dispatcher::new();
    dispatcher
        .include_router(handlers::start::router())
        .include_router(handlers::echo::router());
    dispatcher.start_polling(bot).await
}
