# Public API map

Baseline: aiogram `3.30.0` at `c1b0353ce3d3f8d70f90469038939a956e9e09f7`, Telegram Bot API `10.2`.

This map covers the 95 Python framework modules outside the generated `types`, `methods` and `enums` trees. “Mapped” means the capability is public in Rust; names and syntax may differ where Python relies on runtime mutation, inheritance or keyword injection.

| Python namespace | Rust mapping | Compatibility notes |
| --- | --- | --- |
| package metadata, `loggers`, `exceptions` | `version`, `tracing`, `error::Error` | Independent port/upstream/Bot API coordinates; typed Telegram HTTP, retry, migration, decode, transport, dispatcher, FSM and utility errors |
| `client.bot`, `client.default` | `Bot`, `BotBuilder`, `DefaultBotProperties` | Typed and forward-compatible requests, token-based equality/hash, cached `get_me`, per-call timeout, selective/all default suppression, downloads and redacted debug output |
| `client.session.*` | reqwest-backed client plus `RequestMiddleware`/`RequestNext` | Custom reqwest clients cover proxy/TLS/timeout/connectors; middleware can mutate, observe or terminally answer requests; JSON/multipart and streaming files are built in |
| `client.telegram` | `TelegramApiServer` | Production, test and custom endpoints plus local Bot API file-path translation |
| `client.context_controller` | owned/cloned `Bot` and explicit `UpdateContext` | Rust ownership replaces mutable context mounting; generated bound helpers build typed methods executed by `Bot` |
| `dispatcher.dispatcher`, `dispatcher.router` | `Dispatcher`, `Router` | Typed and raw-JSON workflow feed, nested propagation, used-update resolution, foreground/webhook feed, multi-bot polling, backoff, concurrency limits, signals and programmatic stop |
| `dispatcher.event.*` | routes, `Filter`, `HandlerFlags`, `ClassHandler`, `skip`/`cancel` | Observer root filters, handler filters, all named event shortcuts, class/stateful handlers and typed capture injection |
| `dispatcher.middlewares.*` | `Middleware`, `OuterMiddleware`, `Next`, `OuterNext`, `EventContext` | Global and event-scoped inner/outer chains, user/chat/thread/business context, error propagation and recursive lifecycle |
| `filters.base`, `filters.logic` | `Filter`, `FnFilter`, `FilterExt`, `all`, `either`, `Not` | Async custom filters and AND/OR/NOT composition |
| `filters.command` | `Command`, `CommandStart`, `CommandMatch` | Multiple string/regex commands, prefixes, mentions, Bot lookup, Unicode casefold, deep-link decoding and typed capture injection |
| `filters.callback_data` | `utils::CallbackData`, callback filters | Typed pack/unpack, separator/size validation and parsed-data injection |
| `filters.chat_member_updated` | member markers/groups/transitions and `ChatMemberUpdatedFilter` | Restricted membership semantics and transition operators are preserved |
| remaining filters and magic-filter adapter | state/content/dependency/error filters plus `MagicField` | Nested attr/item access, truthiness, selectors, transforms, comparisons, arithmetic/bitwise operations, custom typed functions and every upstream regex mode |
| `fsm.state`, `fsm.strategy`, `fsm.context` | `State`, `StatesGroup`, `states_group!`, `FsmStrategy`, `FsmContext` | Native macros/builders replace Python metaclasses; manual and update-derived contexts support all storage operations and destinies |
| `fsm.storage.*` | `Storage`, `MemoryStorage`, `RedisStorage`, `MongoStorage`, `KeyBuilder` | Async Rust MongoDB covers the capability of both Python Mongo adapters; state/data TTL and live conformance are covered |
| FSM event isolation | `DisabledEventIsolation`, `SimpleEventIsolation`, `RedisEventIsolation` | Redis leases use token-safe atomic release; middleware closes storage/isolation on dispatcher shutdown |
| `fsm.scene` | `SceneBuilder`, `SceneDefinition`, `SceneRegistry`, `ScenesManager`, `SceneWizard`, `HistoryManager`, `After` | Builder composition replaces class inheritance/decorators while preserving observers, actions, enter/leave hooks, history and navigation |
| `handlers.*` | `ClassHandler` plus `UpdateContext` accessors | One trait covers every specialized Python base handler; message command data and event/error objects are typed dependencies |
| `utils.backoff`, callback answer and chat action | `utils::backoff`, `CallbackAnswerMiddleware`, `ChatActionSender`/middleware | Backoff schedule, handler flags, mutation, periodic actions and cancellation-safe cleanup |
| `utils.keyboard`, `utils.media_group` | inline/reply keyboard builders and `MediaGroupBuilder` | Row adjustment/attachment and caption propagation with recursive upload discovery |
| formatting, markdown and text decorations | `utils::formatting` | UTF-16 entity construction/extraction, nested entity unparse, HTML and MarkdownV2 escaping/decorations, lists and sections |
| links, deep linking and payload | `utils::link`, `utils::deep_linking`, `utils::payload` | Telegram/docs links, start/startgroup/startapp flows and Base64URL/custom payload codecs |
| serialization | `utils::serialization`, serde | Telegram object/method serialization with default application and forward-compatible unknown fields |
| token, auth widget, WebApp and signatures | `utils::token`, `utils::web_app` | Token validation, Login Widget HMAC, WebApp HMAC and Ed25519 third-party validation/parsing |
| `utils.i18n.*` | `i18n` | GNU MO discovery/reload, locale fallback, plural rules, lazy values and simple/constant/FSM middleware |
| Python-only mixin, dataclass, MRO resolver, mypy and warning helpers | native Rust traits, derives, macros, compiler diagnostics and `tracing` | These modules support Python's runtime/type-system mechanics; their user-visible capabilities are supplied by Rust language mechanisms |
| `webhook.security` | `IpFilter`, constant-time secret verification | Telegram CIDRs, custom IPv4/CIDR allow-list and proxy address resolution |
| `webhook.aiohttp_server` | framework-neutral webhook functions and `axum_integration` | Foreground/background dispatch, direct JSON/multipart methods, single/multi-bot routing and graceful live server lifecycle |

Generated surface is checked separately: 390 entities, 35 unions, 38 enums, 185 methods, 185 `Bot` entry points, 187 object aliases, 1,896 mapped type annotations, 980 mapped method annotations and 41 `Default(...)` field mappings. CI regenerates from the pinned checkout and rejects drift.

The hand-written framework surface is also pinned structurally: the compatibility gate inventories 168 public classes, 71 public functions and 531 public methods, hashes their fully-qualified names, and rejects silent upstream drift. Every one of the 770 unique symbols is additionally assigned to an exact module route in [`compatibility/manual-api-routes.toml`](../compatibility/manual-api-routes.toml). The gate requires all 77 modules to be routed exactly once, checks every referenced evidence file, and fingerprints the expanded symbol-to-Rust mapping. A changed upstream or route fingerprint therefore requires an explicit compatibility review.
