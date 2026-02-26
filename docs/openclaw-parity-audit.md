# OpenClaw Parity Audit

Date: 2026-02-25
Scope compared: `openclaw/src/gateway/**` and `openclaw/src/channels/plugins/**` against `reclaw-server/src/**`.

## Implemented parity slices

- Gateway method surface from `server-methods-list.ts` is wired in Rust dispatcher (`src/rpc/dispatcher.rs`).
- Channel HTTP ingress is implemented:
  - `POST /channels/inbound`
  - `POST /channels/{channel}/inbound`
  - Telegram webhook adapter (`POST /channels/telegram/webhook`)
- Hooks ingress baseline is implemented with strict token auth:
  - `POST <hooksPath>/wake`
  - `POST <hooksPath>/agent`
  - policy controls for request `sessionKey` usage and default session/agent IDs
  - static `hooksMappings` path dispatch for mapped wake/agent actions
- OpenAI/OpenResponses compatibility routes are implemented with gateway auth:
  - `POST /v1/chat/completions`
  - `POST /v1/responses`
  - Both are config-gated and disabled by default.
- Static daemon-style config layering is implemented (`/etc/reclaw`, `~/.reclaw`, env, CLI precedence).

## Remaining parity backlog

## P0 (core behavior gaps)

1. Real agent runtime parity (currently stubbed chat semantics).
- OpenClaw references:
  - `src/gateway/server-chat.ts`
  - `src/gateway/openai-http.ts`
  - `src/gateway/openresponses-http.ts`
  - `src/gateway/agent-event-assistant-text.ts`
- Current Rust status:
  - `chat.send` uses deterministic echo response and does not run full agent/tool execution pipeline.
  - run/idempotency reuse is now guarded for `chat.send` and `agent` to avoid duplicate side effects.
  - `agent` supports queued deferred runs executed through `agent.wait` (lifecycle scaffolding).
  - `chat.abort` cancels queued/running deferred runs within the same session.
  - `chat.abort` supports session-wide cancellation when `runId` is omitted.

2. Plugin/channel runtime parity (dynamic channel plugin system).
- OpenClaw references:
  - `src/channels/plugins/**`
  - `src/gateway/server-plugins.ts`
  - `src/gateway/server-channels.ts`
- Current Rust status:
  - Core supports plugin-owned webhook route dispatch via static `channelWebhookPlugins`
    HTTP bridge fallback on `POST /channels/{channel}/webhook`.
  - In-process registry injection remains supported for compiled adapters.
  - `channels.status` now includes configured plugin channels, account-aware summary views, and persisted logout state merge.
  - Plugin account lifecycle and dynamic process/plugin loader parity are still not implemented.

## P1 (high-impact platform parity)

1. Control UI and canvas host HTTP/WS surfaces.
- OpenClaw references:
  - `src/gateway/control-ui.ts`
  - `src/gateway/server-http.ts` (control UI + canvas branches)
  - `src/canvas-host/**`
- Current Rust status:
  - Not implemented.

2. HTTP tools invoke and Slack HTTP handling.
- OpenClaw references:
  - `src/gateway/tools-invoke-http.ts`
  - `src/slack/http/**`
- Current Rust status:
  - Not implemented.

3. Advanced gateway auth model parity for HTTP/WS.
- OpenClaw references:
  - `src/gateway/auth.ts`
  - `src/gateway/auth-rate-limit.ts`
  - `src/gateway/origin-check.ts`
  - `src/gateway/server.auth.test.ts`
- Current Rust status:
  - Auth mode + rate limiting exist, but no origin policy/trusted-proxy/tailscale/canvas-capability equivalent.

## P2 (operational parity)

1. Runtime hot-reload/restart planning and sentinel behavior.
- OpenClaw references:
  - `src/gateway/config-reload.ts`
  - `src/gateway/server-reload-handlers.ts`
  - `src/gateway/server-restart-sentinel.ts`

2. Full OpenResponses feature envelope (input file/image URL processing, tool_choice semantics).
- OpenClaw references:
  - `src/gateway/openresponses-http.ts`
  - `src/gateway/open-responses.schema.ts`
- Current Rust status:
  - Baseline JSON/SSE compatibility exists, but advanced media/tool semantics are not implemented.

3. Event broadcasting depth and runtime subscriptions parity.
- OpenClaw references:
  - `src/gateway/server-broadcast.ts`
  - `src/gateway/server-node-events.ts`
- Current Rust status:
  - Protocol event names are exposed, but OpenClaw-level broadcast fanout semantics are not fully matched.

## Recommended next implementation order

1. Agent runtime upgrade (replace echo path with real execution + stream events).
2. Channel plugin runtime abstraction and plugin account/process lifecycle.
3. Control UI/canvas compatibility surfaces.
4. Advanced OpenResponses media/tool parity.
