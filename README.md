# Starframe (working title)

A tick‑based 4X space strategy sandbox: manage an empire across a large universe, design ships in a block‑based editor, and issue orders that resolve between simulation ticks. Inspired by Aurora 4X (deep simulation), Distant Worlds (macro control), and Space Engineers (block‑based ship design) — but **turn/tick‑based, not real‑time**.

## Tech stack

| Layer | Tech |
|---|---|
| Server | [SpacetimeDB](https://spacetimedb.com) module — **Rust** (world state + deterministic tick) |
| Client | **Rust** — `eframe` / `egui` (UI) + `wgpu` (3D ship editor & maps) |
| Shared | `starframe-shared` crate — enums + pure game math, compiled into both sides |

## Architecture at a glance

- **The server is the only writer.** The client issues reducer calls and renders state streamed back via subscriptions.
- **The tick is a pure, deterministic function** of state + queued orders — golden‑testable, and ready to run on a scheduled timer later.
- **Single‑player now, architected to scale to a shared persistent universe** — all state is identity‑addressed and faction‑scoped from day one.

## Documentation

- **[Technical Design Document](docs/TDD.md)** — the implementation‑ready spec: workspace layout, SpacetimeDB tables & reducers, the tick pipeline, the ship‑editor data flow, the egui/wgpu client, milestones, and tests.

## Workspace layout

```text
starframe/
├── server/   # starframe-server — SpacetimeDB module: tables + reducers (wasm)   ✅ initialized
├── client/   # starframe-client — eframe/egui desktop app                         ✅ scaffolded
└── shared/   # starframe-shared — enums, formulas, pure sim algorithms            ⏳ planned
```

`server` is a wasm-only module and is kept out of the workspace's default host
build (`default-members = ["client"]`), so `cargo build` at the root builds the
client. Build the module explicitly with `spacetime build -p server`.

## Local development & hosting

Prerequisites: the Rust toolchain, the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`), and the SpacetimeDB CLI
(`curl -sSf https://install.spacetimedb.com | sh`).

```sh
# 1. Build / run the desktop client (native window)
cargo run -p starframe-client

# 2. Build the server module to wasm (validates the module)
spacetime build -p server

# 3. Host locally: start a local SpacetimeDB instance, then publish the module
sh scripts/start-local.sh                 # terminal 1 — runs on 127.0.0.1:3000
sh scripts/publish-local.sh               # terminal 2 — publishes as "space4x"

# 4. Drive and inspect the seeded universe
spacetime call space4x create_faction 'Terran Union'
spacetime call space4x advance_tick
spacetime sql  space4x "SELECT * FROM faction"
spacetime logs space4x
```

`spacetime.json` pins the project to the `local` server with `module-path: ./server`.

## Status

Milestone 1 in progress. `init` seeds a small galaxy (8 systems + planets, one AI
faction); `create_faction` lets a player claim a home system; `advance_tick` runs
the economy step and advances the deterministic clock. Movement, combat, ship
building, and the `shared` crate land next (see [docs/TDD.md](docs/TDD.md) §10).
