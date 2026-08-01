use std::sync::Arc;

use aiogram::filters::FilterExt;
use aiogram::fsm::{FsmContext, FsmMiddleware, MemoryStorage};
use aiogram::{Bot, Dispatcher, Result, Router, filters, states_group};

states_group! {
    pub Form {
        NAME => "name",
        AGE => "age",
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let storage = Arc::new(MemoryStorage::default());
    let mut router = Router::named("form");

    router.message(filters::command("form"), |context| async move {
        context
            .dependency::<FsmContext>()
            .expect("FSM middleware is installed")
            .set_state(Form::NAME.clone())
            .await?;
        context.answer("What is your name?").await?;
        Ok(())
    });

    router.message(
        filters::StateFilter::new(Form::NAME.clone()).and(filters::any()),
        |context| async move {
            let name = context
                .message()
                .and_then(|message| message.text.as_deref())
                .unwrap_or("friend");
            context.answer(format!("Nice to meet you, {name}!")).await?;
            context
                .dependency::<FsmContext>()
                .expect("FSM middleware is installed")
                .clear()
                .await
        },
    );

    let mut dispatcher = Dispatcher::new();
    dispatcher
        .include_router(router)
        .fsm(FsmMiddleware::new(storage));
    dispatcher.start_polling(bot).await
}
