# Reclaw Core

`Reclaw Core` is the Rust gateway runtime for Reclaw, forked from and protocol-compatible with [OpenClaw](https://github.com/openclaw/openclaw).

## Specs

- Architecture: `docs/spec/architecture.md`
- Storage: `docs/spec/storage.md`
- Method contracts: `docs/spec/methods.md`

## Run

```bash
cargo run -p reclaw-core -- --host 127.0.0.1 --port 18789
```

## Init Config

Initialize base static config files for daemon or user runtime:

```bash
cargo run -p reclaw-core -- init-config
```

Initialize both `/etc/reclaw` and `~/.reclaw` in one run:

```bash
cargo run -p reclaw-core -- init-config --scope both
```

Run without prompts and overwrite existing config files:

```bash
cargo run -p reclaw-core -- init-config --scope both --non-interactive --force
```

## Static Config

Reclaw Core loads static runtime config from files before applying CLI/env overrides.

Default search order:

1. `/etc/reclaw/config.toml`
2. `/etc/reclaw/config.json`
3. `~/.reclaw/config.toml`
4. `~/.reclaw/config.json`

User-level config overrides `/etc` config, and CLI/env override both.

You can use one explicit file path with:

```bash
reclaw-core --config /etc/reclaw/config.toml
```

or:

```bash
RECLAW_CONFIG=/etc/reclaw/config.toml reclaw-core
```

## Quality Gates

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## Endpoints

- WebSocket: `/` and `/ws`
- Health: `/healthz`
- Readiness: `/readyz`
- Info: `/info`
- Channel ingress: `POST /channels/inbound`
- Channel-specific ingress: `POST /channels/{channel}/inbound`
- Telegram webhook: `POST /channels/telegram/webhook`
- OpenAI chat completions: `POST /v1/chat/completions` (disabled by default)
- OpenResponses: `POST /v1/responses` (disabled by default)

Handshake protocol version: `3`.

## Messenger Integration

### Generic Channel Bridge

`/channels/inbound` lets external adapters (Telegram/Discord/Slack daemons) push messages into
Reclaw Core sessions.

Request body:

```json
{
  "channel": "telegram",
  "conversationId": "123456",
  "text": "hello",
  "agentId": "main",
  "messageId": "m1"
}
```

If `channelsInboundToken` is configured, send `Authorization: Bearer <token>`.

### Telegram Webhook

Set these config keys (or env vars):

- `telegramWebhookSecret` / `RECLAW_TELEGRAM_WEBHOOK_SECRET`
- `telegramBotToken` / `RECLAW_TELEGRAM_BOT_TOKEN` (optional for outbound replies)
- `telegramApiBaseUrl` / `RECLAW_TELEGRAM_API_BASE_URL` (defaults to `https://api.telegram.org`)

Send webhook updates to:

- `POST /channels/telegram/webhook`
- Header: `x-telegram-bot-api-secret-token: <telegramWebhookSecret>`

Text updates are ingested into session keys shaped like:

- `agent:main:telegram:chat:<chat_id>`

## LLM Compatibility Endpoints

These HTTP endpoints are compatible with OpenClaw gateway behavior and are disabled by default.

Enable with static config or env:

- `openaiChatCompletionsEnabled = true` or `RECLAW_OPENAI_CHAT_COMPLETIONS_ENABLED=true`
- `openresponsesEnabled = true` or `RECLAW_OPENRESPONSES_ENABLED=true`

Both routes use the gateway auth mode (`gatewayToken` or `gatewayPassword`) and expect:

- `Authorization: Bearer <secret>`
