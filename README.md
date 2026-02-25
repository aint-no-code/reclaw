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

Handshake protocol version: `3`.
