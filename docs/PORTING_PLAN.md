# aiogram Rust port plan

## Definition of full parity

Full parity means that every public aiogram subsystem has a documented Rust equivalent, every Telegram Bot API 10.2 method and type can be represented and executed (including file uploads), upstream examples have equivalent runnable Rust examples, and a compatibility matrix plus contract tests make omissions visible.

Idiomatic Rust APIs may differ syntactically from Python decorators and runtime dependency injection, but must preserve the capability and ergonomic intent.

## Layers

1. **Generated Bot API layer** — objects, tagged unions, enums, method payloads, return types, multipart metadata, docs, and schema coverage tests.
2. **Client layer** — sessions, defaults, typed execution, files, retries, custom Bot API servers, middleware, and errors.
3. **Dispatch layer** — routers, observers, filters, handler propagation, middleware, dependency injection, update isolation, polling, and webhook responses.
4. **State layer** — FSM strategies, scenes, events isolation, memory/Redis storage, key builders, and storage conformance tests.
5. **Utilities** — callback data, keyboard builders, text formatting, deep links, WebApp validation, i18n, media groups, chat actions, serialization, and markdown/html helpers.
6. **Operations** — webhooks, Axum integration, graceful shutdown, logging/tracing, CI, examples, docs, crates.io metadata, and GitHub releases.

## Compatibility policy

- The crate follows independent SemVer (`port.version`).
- `upstream.aiogram.version` and exact commit identify the Python behavior baseline.
- `upstream.telegram_bot_api.version` identifies the generated API surface.
- A Bot API-only update increments the crate minor version while pre-1.0.
- A breaking ergonomic/API redesign increments the crate minor version pre-1.0 and major version after 1.0.
- Patch releases never change the pinned upstream feature surface.
- Every release tag contains `compatibility.toml`; release notes list both upstream coordinates.

## Milestones

- [x] M0: crate skeleton, compatibility manifest, typed core client, router, filters, in-memory FSM, echo example.
- [x] M1: generator plus 100% schema type/method inventory and JSON fixtures.
- [x] M2: multipart uploads, response/error parity, client defaults and middleware.
- [x] M3: dispatcher observer graph, nested routers, middleware, dependency injection parity, event isolation.
- [x] M4: full filters, magic-filter DSL, callback data and keyboard builders.
- [x] M5: FSM strategies, scenes, Redis/MongoDB storage, live storage contract suite.
- [x] M6: webhook adapters/replies, Axum integration and live HTTP lifecycle contract.
- [x] M7: utilities, i18n, WebApp validation and class-style handlers.
- [x] M8: upstream example parity, contract tests, public API map, docs and release automation.
- [ ] M9: `1.0.0` and the final full-parity declaration. GitHub and crates.io publication are complete for the `0.1.x` parity-candidate line.

## Current implemented surface

- Deterministic generation: 390 entity definitions, 35 aiogram unions, 38 enums, 185 method payloads, direct `Bot` entry points for all 185 methods, and all 187 upstream bound aliases from aiogram `3.30.0` / Bot API `10.2`; all 1,896 type and 980 method annotations are read from aiogram's final generated Python layer.
- Typed JSON and multipart execution, typed union responses, recursive byte/filesystem/URL uploads, default suppression, per-call timeouts, complete Telegram error categories, production/test/local Bot API servers and request middleware.
- All named update observers, nested routers, observer root filters, global/event-scoped inner/outer middleware, typed dependency injection, class handlers, error/recursive lifecycle observers, propagation control, multi-bot long polling, programmatic stop and graceful shutdown.
- FSM strategies, state groups/filter/middleware, key builder, memory and optional Redis/MongoDB storage, manual context resolution, disabled/simple/distributed Redis isolation, scene history/navigation, state-gated observer builders, lifecycle hooks and after-actions; Redis/MongoDB live contracts run in CI.
- Framework-neutral foreground/background webhook feed plus optional Axum adapter/runner, JSON/multipart direct replies, constant-time secret validation, Telegram IP filtering, cached token-based multi-bot routing and live TCP/HTTP lifecycle coverage.
- Magic fields with nested attribute/item selectors, Python truthiness and numeric equality, Unicode casefold, arithmetic/bitwise transforms, collection selectors, typed custom functions/casts, all regex modes and typed captures; callback data, typed handler flags, callback-answer/chat-action middleware, keyboard/media-group builders, composable UTF-16 entity formatting with HTML/MarkdownV2 reconstruction, Telegram/docs/deep links, HMAC/Ed25519 WebApp and Login Widget validation, GNU MO catalogs with locale plural rules, simple/constant/FSM i18n middleware, escaping helpers, and all 15 `Message.send_copy` branches.
- All 14 logical workflows from the pinned upstream example tree mapped to 19 compiling Cargo examples, plus a reproducible crate package and tag-driven GitHub release gate tied to the port/upstream compatibility coordinates.

Unchecked milestone boxes indicate that at least one required parity item in that milestone still remains; they are not a statement that the listed implemented pieces are absent.
