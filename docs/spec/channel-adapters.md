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

If `{channel}` has no registered adapter, core returns `404` with `error.code = "NOT_FOUND"`.

## Injection Model

Router boot supports runtime registry injection:

- `build_router(state)`
  - Uses default registry.
- `build_router_with_webhooks(state, registry)`
  - Uses caller-provided registry.

This allows external crates to register channel adapters and compose the runtime without modifying core dispatch code.

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
