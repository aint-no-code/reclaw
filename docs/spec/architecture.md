# Architecture Specification

## Boundaries

- `domain`: pure domain types, invariants, and domain errors.
- `application`: use-case orchestration, no transport or SQL syntax.
- `storage`: SQLite persistence and repository implementations.
- `interfaces`: HTTP/WS transport and RPC wiring.
- `protocol`: frame and payload contracts.
- `security`: auth and rate limiting.

## Non-negotiable Rules

- SQLite base tables are the source of truth for persistent state.
- Secondary indexes are derived from base rows and never authoritative.
- No in-memory adapter as a substitute for persistent behavior.
- Public RPC behavior is contract-driven and covered by conformance tests.
- Unsupported but known methods return `UNAVAILABLE`; unknown methods return `INVALID_REQUEST`.

## Runtime Modes

- Operator clients use `role=operator`.
- Node clients use `role=node`.
- `connect` must be the first request frame.

## Contracts

- Protocol version: `3`.
- Request frame: `{ type: "req", id, method, params? }`.
- Response frame: `{ type: "res", id, ok, payload?, error? }`.
