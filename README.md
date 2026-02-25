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

Handshake protocol version: `3`.
