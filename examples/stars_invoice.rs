use aiogram::filters::FilterExt;
use aiogram::methods::RefundStarPayment;
use aiogram::types::LabeledPrice;
use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::named("stars-invoice");

    router.message(filters::command("start"), |context| async move {
        let message = context.message().expect("message observer");
        let invoice = message.answer_invoice(
            "Demo invoice",
            "Demo invoice description",
            "demo",
            "XTR",
            vec![LabeledPrice::new("Demo", 42)],
        )?;
        context.bot.execute(&invoice).await?;
        Ok(())
    });

    router.pre_checkout_query(
        filters::field("invoice_payload").equals("demo"),
        |context| async move {
            let query = context
                .update
                .pre_checkout_query
                .as_ref()
                .expect("pre-checkout observer");
            context.bot.execute(&query.answer(true)?).await?;
            Ok(())
        },
    );

    router.message(
        filters::field("successful_payment")
            .exists()
            .and(filters::any()),
        |context| async move {
            let message = context.message().expect("message observer");
            let payment = message
                .successful_payment
                .as_ref()
                .expect("successful_payment filter");
            let user_id = message
                .from_user
                .as_ref()
                .expect("payments have a sender")
                .id;
            context
                .bot
                .execute(&RefundStarPayment::new(
                    user_id,
                    payment.telegram_payment_charge_id.clone(),
                ))
                .await?;
            context
                .answer("Thanks. Your payment has been refunded.")
                .await?;
            Ok(())
        },
    );

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
