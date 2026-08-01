use std::collections::BTreeMap;

use aiogram::i18n::{I18n, I18nContext, I18nMiddleware};
use aiogram::{Bot, Dispatcher, Result, Router, filters};

#[tokio::main]
async fn main() -> Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let locale_path = std::env::var("LOCALES_PATH").unwrap_or_else(|_| "locales".to_owned());
    let i18n = I18n::from_path(locale_path, "en", "messages")?;
    let mut router = Router::named("i18n");
    router.middleware(I18nMiddleware::new(i18n));

    router.message(filters::command("items"), |context| async move {
        let translations = context
            .dependency::<I18nContext>()
            .expect("i18n middleware is installed");
        let text = translations.ngettext_with_plural("{n} item", "{n} items", 3, &BTreeMap::new());
        context.answer(text).await?;
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
