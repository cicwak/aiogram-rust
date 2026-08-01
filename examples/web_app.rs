#[cfg(feature = "webhook-axum")]
use std::collections::BTreeMap;
#[cfg(feature = "webhook-axum")]
use std::sync::Arc;

#[cfg(feature = "webhook-axum")]
use aiogram::methods::SetChatMenuButton;
#[cfg(feature = "webhook-axum")]
use aiogram::types::{
    InlineQueryResultArticle, InputTextMessageContent, MenuButtonWebApp, WebAppInfo,
};
#[cfg(feature = "webhook-axum")]
use aiogram::utils::web_app;
#[cfg(feature = "webhook-axum")]
use aiogram::{Bot, Dispatcher, Result, Router, filters};
#[cfg(feature = "webhook-axum")]
use axum::extract::{Form, State};
#[cfg(feature = "webhook-axum")]
use axum::http::StatusCode;
#[cfg(feature = "webhook-axum")]
use axum::response::{Html, IntoResponse};
#[cfg(feature = "webhook-axum")]
use axum::routing::{get, post};

#[cfg(feature = "webhook-axum")]
#[derive(Clone)]
struct WebState {
    bot: Bot,
    token: Arc<str>,
}

#[cfg(feature = "webhook-axum")]
async fn demo() -> Html<&'static str> {
    Html(
        r#"<!doctype html><button onclick="Telegram.WebApp.sendData('hello')">Send data</button>
<script src="https://telegram.org/js/telegram-web-app.js"></script>"#,
    )
}

#[cfg(feature = "webhook-axum")]
async fn check_data(
    State(state): State<WebState>,
    Form(fields): Form<BTreeMap<String, String>>,
) -> impl IntoResponse {
    let valid = fields
        .get("_auth")
        .is_some_and(|data| web_app::check_signature(&state.token, data));
    let status = if valid {
        StatusCode::OK
    } else {
        StatusCode::UNAUTHORIZED
    };
    (status, axum::Json(serde_json::json!({"ok": valid})))
}

#[cfg(feature = "webhook-axum")]
async fn send_message(
    State(state): State<WebState>,
    Form(fields): Form<BTreeMap<String, String>>,
) -> impl IntoResponse {
    let Some(auth) = fields.get("_auth") else {
        return StatusCode::UNAUTHORIZED;
    };
    let Ok(data) = web_app::safe_parse_init_data(&state.token, auth) else {
        return StatusCode::UNAUTHORIZED;
    };
    let Some(query_id) = data.query_id else {
        return StatusCode::BAD_REQUEST;
    };
    let result = InlineQueryResultArticle::new(
        query_id.clone(),
        "Demo",
        InputTextMessageContent::new("Hello, World!"),
    );
    match state.bot.answer_web_app_query(query_id, result).await {
        Ok(_) => StatusCode::OK,
        Err(error) => {
            tracing::error!(%error, "answerWebAppQuery failed");
            StatusCode::BAD_GATEWAY
        }
    }
}

#[cfg(feature = "webhook-axum")]
#[tokio::main]
async fn main() -> Result<()> {
    let token = std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required");
    let base_url = std::env::var("APP_BASE_URL").expect("APP_BASE_URL is required");
    let bot = Bot::new(token.clone())?;

    bot.set_webhook(format!("{base_url}/webhook")).await?;
    bot.set_chat_menu_button(SetChatMenuButton::new().menu_button(MenuButtonWebApp::new(
        "Open Menu",
        WebAppInfo::new(format!("{base_url}/demo")),
    )))
    .await?;

    let mut bot_router = Router::named("web-app-bot");
    let route_base_url = base_url.clone();
    bot_router.message(filters::command("webview"), move |context| {
        let route_base_url = route_base_url.clone();
        async move {
            context
                .answer(format!("Open the WebApp at {route_base_url}/demo"))
                .await?;
            Ok(())
        }
    });
    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(bot_router);

    let web_routes = axum::Router::new()
        .route("/demo", get(demo))
        .route("/demo/checkData", post(check_data))
        .route("/demo/sendMessage", post(send_message))
        .with_state(WebState {
            bot: bot.clone(),
            token: Arc::from(token),
        });
    let app = web_routes.merge(aiogram::webhook::axum_integration::router(
        "/webhook",
        Arc::new(dispatcher),
        bot,
        std::env::var("WEBHOOK_SECRET").ok(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8081").await?;
    axum::serve(listener, app)
        .await
        .map_err(|error| aiogram::Error::Handler(error.to_string()))
}

#[cfg(not(feature = "webhook-axum"))]
fn main() {
    eprintln!("enable --features webhook-axum to run this example");
}
