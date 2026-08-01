use std::sync::Arc;

use aiogram::fsm::{
    FsmMiddleware, MemoryStorage, SceneAction, SceneBuilder, SceneRegistry, SimpleEventIsolation,
    StateData,
};
use aiogram::{Bot, Dispatcher, Result, Router, filters};
use serde_json::Value;

const QUESTIONS: [(&str, &str); 3] = [
    ("What is the capital of France?", "Paris"),
    ("What is the capital of Spain?", "Madrid"),
    ("What is the capital of Germany?", "Berlin"),
];

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let registry = SceneRegistry::new();

    registry.register(
        SceneBuilder::new("quiz")
            .action(
                SceneAction::Enter,
                "message",
                |context, scenes| async move {
                    let step = scenes
                        .get_value("step")
                        .await?
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as usize;
                    if step == 0 {
                        context.answer("Welcome to the quiz!").await?;
                    }
                    context
                        .answer(format!(
                            "{}\nReply with an answer, `🔙 Back`, or `🚫 Exit`.",
                            QUESTIONS[step].0
                        ))
                        .await?;
                    Ok(())
                },
            )
            .action(SceneAction::Exit, "message", |context, scenes| async move {
                let answers = scenes
                    .get_value("answers")
                    .await?
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default();
                let correct = answers
                    .iter()
                    .enumerate()
                    .filter(|(step, answer)| {
                        answer.as_str() == QUESTIONS.get(*step).map(|question| question.1)
                    })
                    .count();
                context
                    .answer(format!(
                        "Quiz finished: {correct}/{} correct.",
                        QUESTIONS.len()
                    ))
                    .await?;
                scenes.clear_data().await
            })
            .message(
                filters::field("text").equals("🔙 Back"),
                |_, scenes| async move {
                    scenes.back().await?;
                    Ok(())
                },
            )
            .message(
                filters::field("text").equals("🚫 Exit"),
                |_, scenes| async move { scenes.exit().await },
            )
            .message(
                filters::field("text").exists(),
                |context, scenes| async move {
                    let answer = context
                        .message()
                        .and_then(|message| message.text.clone())
                        .unwrap_or_default();
                    let step = scenes
                        .get_value("step")
                        .await?
                        .and_then(|value| value.as_u64())
                        .unwrap_or(0) as usize;
                    let mut answers = scenes
                        .get_value("answers")
                        .await?
                        .and_then(|value| value.as_array().cloned())
                        .unwrap_or_default();
                    if answers.len() == step {
                        answers.push(Value::from(answer));
                    } else if step < answers.len() {
                        answers[step] = Value::from(answer);
                    }
                    let next = step + 1;
                    scenes
                        .update_data(StateData::from([
                            ("step".to_owned(), Value::from(next)),
                            ("answers".to_owned(), Value::Array(answers)),
                        ]))
                        .await?;
                    if next == QUESTIONS.len() {
                        scenes.exit().await
                    } else {
                        scenes.retake().await
                    }
                },
            )
            .build(),
    )?;

    let entry_registry = registry.clone();
    let mut entry = Router::named("quiz-entry");
    entry.message(filters::command("quiz"), move |context| {
        let entry_registry = entry_registry.clone();
        async move {
            let manager = entry_registry.manager(&context)?;
            manager
                .set_data(StateData::from([
                    ("step".to_owned(), Value::from(0)),
                    ("answers".to_owned(), Value::Array(Vec::new())),
                ]))
                .await?;
            manager.enter(Some("quiz")).await
        }
    });

    let storage = Arc::new(MemoryStorage::default());
    let mut dispatcher = Dispatcher::new();
    dispatcher
        .fsm(FsmMiddleware::new(storage).event_isolation(SimpleEventIsolation::default()))
        .include_router(entry)
        .include_router(registry.router()?);
    dispatcher.start_polling(bot).await
}
