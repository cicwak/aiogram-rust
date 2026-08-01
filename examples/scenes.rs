use std::sync::Arc;

use aiogram::fsm::{After, FsmMiddleware, MemoryStorage, SceneAction, SceneBuilder, SceneRegistry};
use aiogram::{Bot, Dispatcher, Result, Router, filters};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("BOT_TOKEN").expect("set BOT_TOKEN before running the example");
    let bot = Bot::new(token)?;
    let registry = SceneRegistry::new();

    registry.register(
        SceneBuilder::new("form:name")
            .message_after(
                filters::any(),
                After::goto("form:language"),
                |context, scenes| async move {
                    let name = context
                        .message()
                        .and_then(|message| message.text.clone())
                        .unwrap_or_default();
                    scenes
                        .update_data([("name".to_owned(), Value::from(name))].into())
                        .await?;
                    Ok(())
                },
            )
            .action(SceneAction::Enter, "message", |context, _| async move {
                context.answer("What is your name?").await?;
                Ok(())
            })
            .build(),
    )?;

    registry.register(
        SceneBuilder::new("form:language")
            .message_after(filters::any(), After::Exit, |context, scenes| async move {
                let name = scenes
                    .get_value("name")
                    .await?
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "friend".to_owned());
                let language = context
                    .message()
                    .and_then(|message| message.text.as_deref())
                    .unwrap_or("unknown");
                context
                    .answer(format!("Nice to meet you, {name}. Language: {language}"))
                    .await?;
                Ok(())
            })
            .action(SceneAction::Enter, "message", |context, _| async move {
                context
                    .answer("What programming language do you prefer?")
                    .await?;
                Ok(())
            })
            .build(),
    )?;

    let entry_registry = registry.clone();
    let mut entry_router = Router::named("scene-entry");
    entry_router.message(filters::command("start"), move |context| {
        let entry_registry = entry_registry.clone();
        async move {
            entry_registry
                .manager(&context)?
                .enter(Some("form:name"))
                .await
        }
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher
        .fsm(FsmMiddleware::new(Arc::new(MemoryStorage::default())))
        .include_router(entry_router)
        .include_router(registry.router()?);
    dispatcher.start_polling(bot).await
}
