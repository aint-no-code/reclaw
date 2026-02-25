# Channel Adapter Specification

This document defines how channel-specific webhook integrations plug into `reclaw-core` without embedding provider-specific logic in the core runtime.

## Goals

- Keep `reclaw-core` transport/runtime stable.
- Isolate channel/provider behavior behind adapter boundaries.
- Allow adding new channel adapters with minimal changes to core.

## Core Contract

`reclaw-core` exposes a webhook adapter registry:

- Type: `ChannelWebhookRegistry`
- Adapter descriptor: `ChannelWebhookAdapter`
- Dispatch function: `WebhookDispatchFn`

Current default registry includes:

- `telegram` adapter
- `discord` adapter
- `slack` adapter
- `signal` adapter
- `whatsapp` adapter

## HTTP Surfaces

- Generic channel webhook dispatch:
  - `POST /channels/{channel}/webhook`
- Backward-compatible legacy Telegram webhook path:
  - `POST /channels/telegram/webhook`

If `{channel}` has no registered in-process adapter, core checks static `channelWebhookPlugins`.
If neither is configured, core returns `404` with `error.code = "NOT_FOUND"`.

## Injection Model

Router boot supports runtime registry injection:

- `build_router(state)`
  - Uses default registry.
- `build_router_with_webhooks(state, registry)`
  - Uses caller-provided registry.

This allows external crates to register channel adapters and compose the runtime without modifying core dispatch code.

## Static Plugin Bridge

Core can proxy unknown channel webhook routes to external plugin daemons:

```toml
[channelWebhookPlugins.extchat]
url = "http://127.0.0.1:4801/webhook"
token = "replace-me" # optional; sent as x-reclaw-plugin-token
timeoutMs = 10000
```

Bridge behavior:

- Incoming webhook payload body is forwarded as-is (`application/json`).
- Incoming headers are forwarded (except host/content-length and reserved reclaw bridge headers).
- Core adds:
  - `x-reclaw-channel: <channel>`
  - `x-reclaw-plugin-token: <token>` when configured.
- Plugin response status and JSON body are passed through to caller.
- Non-JSON plugin responses are rejected with `502 BAD_GATEWAY`.
- `channels.status` includes configured plugin channels (`kind = "plugin"`) and reflects persisted logout state.

## Adapter Rules

- Adapter logic must validate channel-specific auth/signatures.
- Adapter logic must treat storage-backed runtime methods as source of truth.
- Adapter failures must return explicit JSON error shapes; no silent drops.
- Adapter code must not bypass core auth/rate-limit invariants for operator/node RPC.
- Non-Telegram adapters can optionally dispatch outbound reply relays using channel-specific
  `*OutboundUrl` + `*OutboundToken` config keys.
- Outbound relay payload shape is normalized:
  - `channel`
  - `conversationId`
  - `reply`
  - `sessionKey`
  - `runId`
  - `sourceSenderId`
  - `sourceMessageId`
  - `metadata` (optional)

## Next Steps

- Move Telegram adapter into `reclaw-telegram` crate and register via injected registry.
- Define a conformance suite in `reclaw-conformance` for adapter behavior:
  - auth rejection
  - idempotent ingest
  - session key shaping
  - outbound delivery semantics
