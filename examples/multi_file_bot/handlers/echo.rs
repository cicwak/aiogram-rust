use aiogram::{Router, filters};

pub fn router() -> Router {
    let mut router = Router::named("echo");
    router.message(filters::any(), |context| async move {
        let copy = context
            .message()
            .expect("message observer")
            .send_copy(context.message().expect("message observer").chat.id);
        match copy {
            Ok(method) => {
                context.bot.execute_send_copy(&method).await?;
            }
            Err(_) => {
                context.answer("Nice try!").await?;
            }
        }
        Ok(())
    });
    router
}
