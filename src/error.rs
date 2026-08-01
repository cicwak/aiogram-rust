use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Extra information returned by Telegram for a failed request.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResponseParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub migrate_to_chat_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

/// Errors surfaced by the Telegram client, dispatcher, and handlers.
#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid bot token: {0}")]
    InvalidToken(String),

    #[error("Telegram API returned {error_code}: {description}")]
    Telegram {
        method: String,
        error_code: u16,
        description: String,
        parameters: Option<ResponseParameters>,
    },

    #[error("Telegram requested a retry for {method} after {retry_after:?}: {description}")]
    RetryAfter {
        method: String,
        retry_after: Duration,
        description: String,
    },

    #[error("Telegram migrated the chat for {method} to {migrate_to_chat_id}: {description}")]
    MigrateToChat {
        method: String,
        migrate_to_chat_id: i64,
        description: String,
    },

    #[error("Telegram rejected {method} as a bad request: {description}")]
    BadRequest { method: String, description: String },

    #[error("Telegram could not find the target for {method}: {description}")]
    NotFound { method: String, description: String },

    #[error("Telegram reported a conflict for {method}: {description}")]
    Conflict { method: String, description: String },

    #[error("Telegram rejected authorization for {method}: {description}")]
    Unauthorized { method: String, description: String },

    #[error("Telegram forbids {method}: {description}")]
    Forbidden { method: String, description: String },

    #[error("Telegram rejected an oversized entity for {method}: {description}")]
    EntityTooLarge { method: String, description: String },

    #[error("Telegram server failed while handling {method}: {description}")]
    Server { method: String, description: String },

    #[error("Telegram is restarting while handling {method}: {description}")]
    Restarting { method: String, description: String },

    #[error("HTTP transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("Telegram response for {method} could not be decoded: {reason}; content: {data}")]
    ClientDecode {
        method: String,
        reason: String,
        data: String,
    },

    #[error("Telegram request {method} timed out after {timeout:?}")]
    RequestTimeout { method: String, timeout: Duration },

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "fsm-redis")]
    #[error("Redis storage error: {0}")]
    Redis(#[from] redis::RedisError),

    #[cfg(feature = "fsm-mongodb")]
    #[error("MongoDB storage error: {0}")]
    Mongo(#[from] mongodb::error::Error),

    #[error("invalid Telegram request payload: {0}")]
    InvalidPayload(String),

    #[error("handler error: {0}")]
    Handler(String),

    #[error("FSM error: {0}")]
    Fsm(String),

    #[error("utility error: {0}")]
    Utility(String),

    #[error("handler requested propagation to continue")]
    SkipHandler,

    #[error("handler cancelled update propagation")]
    CancelHandler,

    #[error("dispatcher stopped")]
    DispatcherStopped,

    #[error("polling is already running")]
    PollingAlreadyStarted,

    #[error("polling is not running")]
    PollingNotStarted,
}
