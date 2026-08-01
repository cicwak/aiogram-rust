use aiogram::dispatcher::UpdateContext;
use aiogram::filters::{Filter, FilterFuture};
use aiogram::{Bot, Dispatcher, Result, Router};

struct ExactText(&'static str);

impl Filter for ExactText {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            context
                .message()
                .and_then(|message| message.text.as_deref())
                == Some(self.0)
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("custom-filter");
    router.message(ExactText("hello"), |context| async move {
        context
            .answer("Hello from a custom Filter implementation")
            .await?;
        Ok(())
    });
    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
