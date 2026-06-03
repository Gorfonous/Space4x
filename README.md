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

# 4. Drive the simulation: issue orders, then advance time (1 day = TICKS_PER_DAY)
spacetime call space4x order_build_ship <design_id> <fleet_id>     # queue a build
spacetime call space4x order_move_fleet <fleet_id> <dest_system_id>
spacetime call space4x advance_days 1
spacetime sql  space4x "SELECT * FROM faction"        # resources
spacetime sql  space4x "SELECT * FROM ship"           # fleets / ships
spacetime sql  space4x "SELECT * FROM combat_event"   # battles
spacetime sql  space4x "SELECT * FROM sim_run"        # tick-batch completion log
spacetime logs space4x

# Design a ship via the draft reducers. Enum tags are camelCase, and you must
# pass `--` so negative block coordinates aren't parsed as CLI flags:
spacetime call space4x create_draft <faction_id> 'Frigate'
spacetime call space4x place_block -- <draft_id> 0 0 0 '{"commandCore": {}}' 0
spacetime call space4x place_block -- <draft_id> 1 0 0 '{"engine": {}}' 0
spacetime call space4x place_block -- <draft_id> -1 0 0 '{"reactor": {}}' 0
spacetime call space4x commit_design <draft_id> 'Frigate'   # validates, then snapshots
```

`spacetime.json` pins the project to the `local` server with `module-path: ./server`.

## Status

Milestones 1–2 done; core simulation (M4) in. `init` seeds the galaxy (8 systems +
planets) with a player and an AI faction, each given a starter Scout design + home
fleet. Each tick the server simulates fleet **movement**, ship **building**, and
deterministic **combat** alongside the economy; `advance_ticks` / `advance_days`
(= days × `TICKS_PER_DAY`) drive time and log a `sim_run` row. The desktop client
connects, renders Empire + Systems from live subscriptions, and currently drives
time (Advance Day / Tick); the build/move/attack reducers exist server-side and get
client UI next (see [docs/TDD.md](docs/TDD.md) §10). The deterministic
`run_tick` does the economy step today; movement, combat, and ship building land
next (see [docs/TDD.md](docs/TDD.md) §10).
