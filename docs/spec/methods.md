# Method Contract Specification

## Implemented Groups

- `health`, `status`
- `config.*`
- `sessions.*`
- `agent`, `agent.wait`, `agent.identity.get`
- `chat.send`, `chat.history`, `chat.abort`
- `cron.list`, `cron.status`, `cron.add`, `cron.update`, `cron.remove`, `cron.run`, `cron.runs`
- `node.pair.request`, `node.pair.list`, `node.pair.approve`, `node.pair.reject`, `node.pair.verify`
- `node.rename`, `node.list`, `node.describe`, `node.invoke`, `node.invoke.result`, `node.event`

## Error Rules

- Invalid request shape or invalid parameter: `INVALID_REQUEST`.
- Known but not implemented: `UNAVAILABLE`.
- Authentication failure: `UNAVAILABLE` with auth-specific message.
- Node pairing violations: `NOT_PAIRED` where applicable.

## Conformance Expectations

- Handshake enforces protocol negotiation and first-frame `connect`.
- `/healthz`, `/readyz`, `/info` must always return JSON.
- Implemented method list in handshake must match dispatcher implementation.
