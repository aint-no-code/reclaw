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

## Runtime Notes

- `agent` accepts optional `deferred=true` to create a queued run that executes when `agent.wait` is called.
- `agent` ensures `sessionKey` exists in session storage before run execution.
- WebSocket clients with connect capability `agent-events-v1` receive server-push `evt` frames for `agent` lifecycle/assistant updates and `chat` final/error updates.
- `chat.abort` cancels queued/running agent runs for the same `sessionKey`.
- `chat.abort` without `runId` cancels all non-terminal runs for the provided `sessionKey`.
- `chat.abort` for completed or unknown runs is a no-op (`aborted == false`) and includes the requested run id in `runIds`.

## Error Rules

- Invalid request shape or invalid parameter: `INVALID_REQUEST`.
- Known but not implemented: `UNAVAILABLE`.
- Authentication failure: `UNAVAILABLE` with auth-specific message.
- Node pairing violations: `NOT_PAIRED` where applicable.

## Conformance Expectations

- Handshake enforces protocol negotiation and first-frame `connect`.
- `/healthz`, `/readyz`, `/info` must always return JSON.
- Implemented method list in handshake must match dispatcher implementation.
