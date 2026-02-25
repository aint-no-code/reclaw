# Storage Specification

## Source of Truth

Persistent state is stored in SQLite. Base tables:

- `config_entries`
- `sessions`
- `chat_messages`
- `agent_runs`
- `cron_jobs`
- `cron_runs`
- `nodes`
- `node_pair_requests`
- `node_invokes`
- `node_events`

## Derived Indexes

Indexes and query orderings are derived from base rows:

- Session lists sorted by `updated_at_ms`.
- Chat history sorted by `ts_ms`.
- Cron runs sorted by `started_at_ms`.
- Node lists sorted by connection/`last_seen_ms`.

## Invariants

- IDs are immutable primary keys.
- JSON blobs are stored as valid JSON text.
- Foreign-key-like references are validated at write boundaries.
- Timestamps are unix milliseconds.
