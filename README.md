# aiogram for Rust

An ergonomic, fully asynchronous Rust port of Python's [aiogram](https://github.com/aiogram/aiogram).
The target is behavioral and feature parity while using idiomatic Rust APIs, static typing, and Tokio.

> Status: parity candidate for the pinned aiogram `3.30.0` baseline. The implementation matrix has no known capability gaps, but `1.0.0` and publication remain gated on the final API/release review.

## Quick start

```rust,no_run
use aiogram::{Bot, Dispatcher, Router, filters};

#[tokio::main]
async fn main() -> aiogram::Result<()> {
    let bot = Bot::new(std::env::var("BOT_TOKEN").expect("BOT_TOKEN is required"))?;
    let mut router = Router::new();

    router.message(filters::command("start"), |context| async move {
        context.answer("Hello from Rust!").await?;
        Ok(())
    });

    let mut dispatcher = Dispatcher::new();
    dispatcher.include_router(router).start_polling(bot).await
}
```

See [`examples/echo_bot.rs`](examples/echo_bot.rs) for a runnable bot.

## Version tracking

The crate has its own SemVer version and separately pins both the Python aiogram revision and Telegram Bot API version it implements. The source of truth is [`compatibility.toml`](compatibility.toml); compiled constants are exposed through `aiogram::COMPATIBILITY`. Tests fail when the crate and compatibility manifest drift.

Current baseline:

- Rust port: `0.1.0`
- aiogram: `3.30.0` at `c1b0353ce3d3f8d70f90469038939a956e9e09f7`
- Telegram Bot API: `10.2` (2026-07-14)

The ignored `aiogram/` checkout is an upstream reference, not part of the Rust crate. It can be restored at the locked commit with `scripts/fetch-upstream.sh`.

## Architecture and parity

- `Bot` exposes all 185 Bot API calls as async methods; typed `TelegramMethod` payloads remain available when optional fields need builder-style configuration, and all 41 upstream default-property aliases are generated rather than guessed from wire names.
- All 390 upstream object definitions, 35 helper unions, 38 enums, 185 Bot API methods, and 187 bound object shortcuts are generated from the pinned aiogram source; `Message::send_copy` selects and executes all 15 aiogram-supported copy variants.
- `Router` and `Dispatcher` provide all named update observers, observer-scoped filters/middleware, typed injection and flags, class handlers, recursive lifecycle, multi-bot polling with programmatic shutdown, and webhooks.
- `filters` contains command, magic-field, text, callback-data, chat-member transition, state, dependency, error, and custom closure filters.
- `fsm` provides strategies, state groups, memory/Redis/MongoDB storage with live CI contracts, local/distributed event isolation, manual context access, history, scene observers/transitions, and managed shutdown.
- Unknown Telegram fields are preserved to allow forward-compatible deserialization.
- Generated sources are reproducible with `cargo run -p xtask -- generate --upstream aiogram`; CI rejects drift from the pinned upstream snapshot.
- Framework behavior is ported manually and checked with contract tests; all 770 public symbols from the hand-written upstream surface have exact native, semantic, or Rust-language routes verified by the compatibility gate.

The detailed work breakdown and completion criteria are in [`docs/PORTING_PLAN.md`](docs/PORTING_PLAN.md).
The current, deliberately conservative subsystem status is in [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md).
The module-by-module Python-to-Rust capability mapping is in [`docs/PUBLIC_API_MAP.md`](docs/PUBLIC_API_MAP.md).

Optional integrations:

- `webhook-axum` — Axum webhook adapter.
- `fsm-redis` — Redis-backed FSM storage with independent state/data TTLs.
- `fsm-mongodb` — MongoDB-backed FSM state/data documents compatible with aiogram's storage layout.

The 19 runnable Cargo examples cover every logical workflow in the pinned
upstream example tree, plus callbacks, handler flags, GNU gettext, bound methods,
and `Message::send_copy`. See [`docs/EXAMPLE_PARITY.md`](docs/EXAMPLE_PARITY.md)
for the explicit mapping.

Tag-based GitHub releases run the complete compatibility gate and attach a verified `.crate` package. crates.io publication remains a separate explicit operation.

## License

MIT. This project is a clean Rust port based on the MIT-licensed aiogram project; see [`NOTICE`](NOTICE) for attribution.
