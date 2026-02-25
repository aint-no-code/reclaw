# Hooks Ingress Specification

This document defines the OpenClaw-compatible external hooks ingress implemented in `reclaw-core`.

## Configuration

- `hooksEnabled` (`RECLAW_HOOKS_ENABLED`)
- `hooksToken` (`RECLAW_HOOKS_TOKEN`)
- `hooksPath` (`RECLAW_HOOKS_PATH`, default `/hooks`)
- `hooksMaxBodyBytes` (`RECLAW_HOOKS_MAX_BODY_BYTES`, default `262144`)
- `hooksAllowRequestSessionKey` (`RECLAW_HOOKS_ALLOW_REQUEST_SESSION_KEY`, default `false`)
- `hooksDefaultSessionKey` (`RECLAW_HOOKS_DEFAULT_SESSION_KEY`, optional)
- `hooksDefaultAgentId` (`RECLAW_HOOKS_DEFAULT_AGENT_ID`, default `main`)
- `hooksMappings` (static config array, optional)

`hooksEnabled=true` requires `hooksToken` to be configured.

## Routes

- `POST <hooksPath>/wake`
- `POST <hooksPath>/agent`

If hooks are disabled, these routes are not mounted.

## Authentication

Accepted tokens:

- `Authorization: Bearer <token>`
- `X-OpenClaw-Token: <token>`

Query parameter auth is rejected (`?token=...`).

## Request Semantics

### Wake

Request body:

```json
{ "text": "wake reason", "mode": "now" }
```

- `text` is required.
- `mode` defaults to `now`.
- `next-heartbeat` mode is accepted and recorded as pending wake metadata.

### Agent

Request body:

```json
{ "message": "hello", "agentId": "main", "sessionKey": "hook:abc" }
```

- `message` is required.
- `agentId` defaults to `hooksDefaultAgentId`.
- `sessionKey` is blocked unless `hooksAllowRequestSessionKey=true`.
- If `sessionKey` is omitted:
  - use `hooksDefaultSessionKey` when configured
  - otherwise generate `hook:<uuid>`

## Responses

- `wake`: `200` with `{ ok, mode }`
- `agent`: `202` with `{ ok, runId, sessionKey, agentId }`
- Invalid payload/policy: `400` with explicit error code/message.
- Invalid/absent token: `401`, rate-limited failures: `429`.

## Mapping Semantics

Paths under `<hooksPath>/<subpath>` can be mapped through `hooksMappings` entries.

Example:

```toml
[[hooksMappings]]
path = "github/push"
action = "agent"
matchSource = "github"
messageTemplate = "repo={{repo}} actor={{actor.name}}"
sessionKey = "hook:github"

[[hooksMappings]]
path = "watchdog/ping"
action = "wake"
textTemplate = "watchdog ping {{source}}"
wakeMode = "next-heartbeat"
```

Rules:

- mapping path compare is normalized (`/` trimming and slash collapsing)
- `matchSource` filters on payload `source`
- `action = "agent"` requires `message` or `messageTemplate`
- `action = "wake"` requires `text` or `textTemplate`
- `messageTemplate` / `textTemplate` support interpolation from:
  - payload (`{{repo}}`, `{{actor.name}}`, `{{commits[0].id}}`)
  - headers (`{{headers.user-agent}}`)
  - query params (`{{query.kind}}`)
  - request subpath (`{{path}}`)
- mapping-provided `sessionKey` is allowed regardless of `hooksAllowRequestSessionKey`
