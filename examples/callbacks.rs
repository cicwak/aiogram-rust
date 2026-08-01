use aiogram::methods::SendMessage;
use aiogram::utils::callback_answer::{CallbackAnswer, CallbackAnswerMiddleware};
use aiogram::utils::keyboard::InlineKeyboardBuilder;
use aiogram::{Bot, Dispatcher, Result, Router, callback_data, filters};

callback_data! {
    struct Action("action") {
        item_id: i64,
        operation: String,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::new();
    router.middleware(CallbackAnswerMiddleware::new());

    router.message(filters::command("start"), |context| async move {
        let data = Action::new(42, "open".to_owned()).pack()?;
        let keyboard = InlineKeyboardBuilder::new()
            .callback("Open item 42", data)?
            .build();
        let message = context.message().expect("message observer");
        context
            .bot
            .execute(&SendMessage::new(message.chat.id, "Choose an action").reply_markup(keyboard))
            .await?;
        Ok(())
    });

    router.callback_query(
        filters::callback_data_filter(Action::unpack, |action| action.operation == "open"),
        |context| async move {
            let action = context.dependency::<Action>().expect("filter injection");
            context
                .dependency::<CallbackAnswer>()
                .expect("callback-answer middleware")
                .text(format!("Opening item {}", action.item_id))?;
            Ok(())
        },
    );

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
