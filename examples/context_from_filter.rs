use aiogram::dispatcher::UpdateContext;
use aiogram::filters::{Filter, FilterFuture};
use aiogram::{Bot, Dispatcher, Result, Router};

#[derive(Debug)]
struct GreetingName(String);

struct HelloFilter {
    name: Option<String>,
}

impl Filter for HelloFilter {
    fn check<'a>(&'a self, context: &'a UpdateContext) -> FilterFuture<'a> {
        Box::pin(async move {
            let Some(message) = context.message() else {
                return false;
            };
            if !message
                .text
                .as_deref()
                .is_some_and(|text| text.eq_ignore_ascii_case("hello"))
            {
                return false;
            }
            let Some(user) = message.from_user.as_ref() else {
                return false;
            };
            let mention = self
                .name
                .as_deref()
                .map_or_else(|| user.mention_html(), |name| user.mention_html_as(name));
            context.inject_dependency(GreetingName(mention));
            true
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("context-from-filter");
    router.message(HelloFilter { name: None }, |context| async move {
        let name = context
            .dependency::<GreetingName>()
            .expect("HelloFilter injects GreetingName");
        context.answer(format!("Hello, {}!", name.0)).await?;
        Ok(())
    });
    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
