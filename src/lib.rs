//! An asynchronous Telegram Bot API framework for Rust.
//!
//! The crate follows aiogram's architecture: [`Bot`] executes typed Telegram
//! methods, [`Router`] selects handlers, [`Dispatcher`] consumes updates, and
//! [`fsm`] provides pluggable finite-state-machine storage.

pub mod bot;
pub mod client;
pub mod dispatcher;
pub mod enums;
pub mod error;
pub mod filters;
pub mod fsm;
pub mod i18n;
pub mod methods;
pub mod types;
pub mod utils;
pub mod version;
pub mod webhook;

pub use bot::{Bot, BotBuilder};
pub use client::{
    BotRequest, DefaultBotProperties, RequestLogging, RequestMiddleware, RequestNext,
    TelegramApiServer,
};
pub use dispatcher::{
    ClassHandler, Dispatcher, EventContext, HandlerFlags, Middleware, Next, OuterMiddleware,
    OuterNext, Router, UpdateContext,
};
pub use error::{Error, Result};
pub use version::{AIogramCompatibility, COMPATIBILITY};
