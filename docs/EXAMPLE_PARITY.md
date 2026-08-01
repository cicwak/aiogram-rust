# Upstream example parity

The pinned aiogram checkout contains 20 Python source files that form 14 runnable
workflows. Each workflow has a compiling Rust counterpart:

| Upstream workflow | Rust example | Notes |
| --- | --- | --- |
| `context_addition_from_filter.py` | `context_from_filter` | A custom filter injects typed handler data |
| `echo_bot.py` | `echo_bot` | Polling, commands, and echo handling |
| `echo_bot_webhook.py` | `webhook_axum` | Axum webhook and direct response support |
| `echo_bot_webhook_ssl.py` | `webhook_axum` | The same application router; production TLS is normally terminated by the reverse proxy in front of Axum |
| `error_handling.py` | `error_handling` | Error observer and typed error context |
| `finite_state_machine.py` | `fsm_form` | FSM storage, state filter, and context |
| `multi_file_bot/*` | `multi_file_bot` | Cargo example directory with separate handler modules |
| `multibot.py` | `multibot_webhook` | Single-bot secret route plus cached token-based multi-bot route |
| `own_filter.py` | `custom_filter` | A user-defined `Filter` implementation |
| `quiz_scene.py` | `quiz_scene` | Scene actions, data, retake/back/exit, and event isolation |
| `scene.py` | `scenes` | Scene registry, transitions, and lifecycle actions |
| `specify_updates.py` | `specify_updates` | Nested routers and inferred `allowed_updates` |
| `stars_invoice.py` | `stars_invoice` | XTR invoice, pre-checkout, payment, and refund |
| `web_app/*` | `web_app` | WebApp menu, signature validation, init-data parsing, and `answerWebAppQuery` |
| `without_dispatcher.py` | `without_dispatcher` | Direct `Bot` usage without polling or a dispatcher |

The crate also includes focused examples for bound methods, callback data,
handler flags, gettext i18n, and `Message::send_copy`, for 19 Cargo example
targets in total.

Run the complete compile contract with:

```console
cargo check --examples --all-features
```
