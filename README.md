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
# 1. Run the desktop client (native window). It connects READ-ONLY to the
#    space4x database at 127.0.0.1:3000 and renders Empire + Systems from live
#    subscriptions; its only commands are the Advance Day / Advance Tick
#    buttons. Start + publish the server first (steps 2-3) so there's data.
cargo run -p starframe-client

# 2. Build the server module to wasm (validates the module)
spacetime build -p server

# 3. Host locally: start a local SpacetimeDB instance, then publish the module
sh scripts/start-local.sh                 # terminal 1 — runs on 127.0.0.1:3000
sh scripts/publish-local.sh               # terminal 2 — publishes as "space4x"

# 4. Advance time and inspect (the client is read-only; advancing N ticks is
#    the only command it sends — here, one day = TICKS_PER_DAY ticks)
spacetime call space4x advance_days 1
spacetime sql  space4x "SELECT * FROM faction"   # resources grew
spacetime sql  space4x "SELECT * FROM sim_run"   # completion signal
spacetime logs space4x
```

`spacetime.json` pins the project to the `local` server with `module-path: ./server`.

## Status

Milestone 1 done; Milestone 2 wired. `init` seeds the galaxy (8 systems + planets)
with a player and an AI faction. The desktop client connects **read‑only**, renders
Empire + Systems from live subscriptions, and drives time with Advance Day / Advance
Tick — its only command is to advance the simulation by N ticks (`advance_ticks`, or
`advance_days` = days × `TICKS_PER_DAY`); the server records a `sim_run` row when done. The deterministic
`run_tick` does the economy step today; movement, combat, and ship building land
next (see [docs/TDD.md](docs/TDD.md) §10).
