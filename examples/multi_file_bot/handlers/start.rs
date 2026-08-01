use aiogram::{Router, filters};

pub fn router() -> Router {
    let mut router = Router::named("start");
    router.message(filters::command("start"), |context| async move {
        let name = context
            .message()
            .and_then(|message| message.from_user.as_ref())
            .map(|user| user.full_name())
            .unwrap_or_else(|| "friend".to_owned());
        context.answer(format!("Hello, {name}!")).await?;
        Ok(())
    });
    router
}
