# Reclaw Core

`Reclaw Core` is the Rust gateway runtime for Reclaw, forked from and protocol-compatible with [OpenClaw](https://github.com/openclaw/openclaw).

## Specs

- Architecture: `docs/spec/architecture.md`
- Channel adapters: `docs/spec/channel-adapters.md`
- Hooks ingress: `docs/spec/hooks.md`
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
- Channel webhook dispatch: `POST /channels/{channel}/webhook`
- Hooks ingress: `POST <hooksPath>/wake` and `POST <hooksPath>/agent` (disabled by default)
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
- Or generic adapter route: `POST /channels/telegram/webhook`
- Header: `x-telegram-bot-api-secret-token: <telegramWebhookSecret>`

Text updates are ingested into session keys shaped like:

- `agent:main:telegram:chat:<chat_id>`

### Additional Channel Webhook Adapters

Built-in webhook adapters are now available for:

- `discord` at `POST /channels/discord/webhook`
- `slack` at `POST /channels/slack/webhook`
- `signal` at `POST /channels/signal/webhook`
- `whatsapp` at `POST /channels/whatsapp/webhook`

Each of these adapters requires a bearer token in `Authorization`:

- `discordWebhookToken` / `RECLAW_DISCORD_WEBHOOK_TOKEN`
- `slackWebhookToken` / `RECLAW_SLACK_WEBHOOK_TOKEN`
- `signalWebhookToken` / `RECLAW_SIGNAL_WEBHOOK_TOKEN`
- `whatsappWebhookToken` / `RECLAW_WHATSAPP_WEBHOOK_TOKEN`

Optional outbound relay can be enabled per channel. When enabled and a reply is produced,
Reclaw Core `POST`s a normalized JSON payload to the configured URL:

- `discordOutboundUrl` / `RECLAW_DISCORD_OUTBOUND_URL`
- `discordOutboundToken` / `RECLAW_DISCORD_OUTBOUND_TOKEN`
- `slackOutboundUrl` / `RECLAW_SLACK_OUTBOUND_URL`
- `slackOutboundToken` / `RECLAW_SLACK_OUTBOUND_TOKEN`
- `signalOutboundUrl` / `RECLAW_SIGNAL_OUTBOUND_URL`
- `signalOutboundToken` / `RECLAW_SIGNAL_OUTBOUND_TOKEN`
- `whatsappOutboundUrl` / `RECLAW_WHATSAPP_OUTBOUND_URL`
- `whatsappOutboundToken` / `RECLAW_WHATSAPP_OUTBOUND_TOKEN`

For external plugin daemons, configure static webhook proxy fallback:

```toml
[channelWebhookPlugins.extchat]
url = "http://127.0.0.1:4801/webhook"
token = "replace-me" # optional, sent as x-reclaw-plugin-token
timeoutMs = 10000
```

With this config, `POST /channels/extchat/webhook` is proxied to the plugin URL when no built-in adapter is registered.

### Hooks Ingress

OpenClaw-compatible `/hooks/*` ingress is available behind explicit config:

- `hooksEnabled` / `RECLAW_HOOKS_ENABLED` (`true` to enable)
- `hooksToken` / `RECLAW_HOOKS_TOKEN` (required when enabled)
- `hooksPath` / `RECLAW_HOOKS_PATH` (default `/hooks`)
- `hooksMaxBodyBytes` / `RECLAW_HOOKS_MAX_BODY_BYTES` (default `262144`)
- `hooksAllowRequestSessionKey` / `RECLAW_HOOKS_ALLOW_REQUEST_SESSION_KEY` (default `false`)
- `hooksDefaultSessionKey` / `RECLAW_HOOKS_DEFAULT_SESSION_KEY` (optional)
- `hooksDefaultAgentId` / `RECLAW_HOOKS_DEFAULT_AGENT_ID` (default `main`)
- `hooksMappings` (static config array for path-based mapped actions)
  - supports `matchSource`, `messageTemplate`, `textTemplate`, and template contexts (`payload`, `headers`, `query`, `path`)

Supported routes once enabled:

- `POST <hooksPath>/wake` body `{ "text": "...", "mode": "now|next-heartbeat" }`
- `POST <hooksPath>/agent` body `{ "message": "...", "agentId"?, "sessionKey"? }`
- `POST <hooksPath>/<custom>` mapped by `hooksMappings` entries

## LLM Compatibility Endpoints

These HTTP endpoints are compatible with OpenClaw gateway behavior and are disabled by default.

Enable with static config or env:

- `openaiChatCompletionsEnabled = true` or `RECLAW_OPENAI_CHAT_COMPLETIONS_ENABLED=true`
- `openresponsesEnabled = true` or `RECLAW_OPENRESPONSES_ENABLED=true`

Both routes use the gateway auth mode (`gatewayToken` or `gatewayPassword`) and expect:

- `Authorization: Bearer <secret>`
