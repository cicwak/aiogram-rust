use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;

    let mut callbacks = Router::named("callbacks-only");
    callbacks.callback_query(filters::any(), |context| async move {
        context.answer_callback(Some("Yeah, good")).await?;
        Ok(())
    });

    let mut edits = Router::named("edited-messages-only");
    edits.edited_message(filters::any(), |context| async move {
        context.reply("Message was edited").await?;
        Ok(())
    });

    let mut root = Router::named("root");
    root.message(filters::command("start"), |context| async move {
        context.answer("Hello!").await?;
        Ok(())
    });
    root.include_router(callbacks).include_router(edits);

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(root);
    let allowed = dispatcher
        .resolve_used_update_types()
        .expect("all example observers have a named update type");
    tracing::info!(?allowed, "requesting only updates used by the router tree");
    dispatcher.allowed_updates(allowed).start_polling(bot).await
}
