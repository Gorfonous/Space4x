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
├── shared/   # starframe-shared — enums + pure game math (ship stats, validate)    ✅
├── server/   # starframe-server — SpacetimeDB module: tables, reducers, tick (wasm) ✅
└── client/   # starframe-client — eframe/egui desktop app (Empire + Systems views)  ✅
```

`server` is a wasm-only module and is kept out of the workspace's default host
build (`default-members = ["client"]`), so `cargo build` at the root builds the
client. Build the module explicitly with `spacetime build -p server`.

## Local development & hosting

Prerequisites: the Rust toolchain, the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`), and the **SpacetimeDB CLI 2.3.0**
(the project is built against 2.3.0 — `spacetime version use 2.3.0`).

### Quick start — one command

In **debug** builds the client bootstraps the backend for you: it starts the
local SpacetimeDB host (if it isn't already running) and publishes the `space4x`
module, then opens the window.

```sh
cargo run -p starframe-client
```

The host keeps running in the background; re-running just re-publishes and
reconnects. Set `STARFRAME_AUTOSTART=0` to skip the bootstrap and run the server
yourself. (Requires the 2.3.0 CLI on PATH — the publish step uses `--module-path`.)

### Manual control

```sh
spacetime build -p server                 # build / validate the module to wasm
sh scripts/start-local.sh                 # terminal 1 — host on 127.0.0.1:3000
spacetime server set-default local
sh scripts/publish-local.sh               # terminal 2 — publishes "space4x"
```

### Driving the simulation by CLI

```sh
spacetime call space4x order_build_ship <design_id> <fleet_id>      # queue a build
spacetime call space4x order_move_fleet <fleet_id> <dest_system_id>
spacetime call space4x advance_days 1                               # 1 day = TICKS_PER_DAY
spacetime sql  space4x "SELECT * FROM faction"        # resources
spacetime sql  space4x "SELECT * FROM ship"           # fleets / ships
spacetime sql  space4x "SELECT * FROM combat_event"   # battles
spacetime sql  space4x "SELECT * FROM sim_run"        # tick-batch completion log

# Design a ship. Enum tags are camelCase, and pass `--` so negative block
# coordinates aren't parsed as CLI flags:
spacetime call space4x create_draft <faction_id> 'Frigate'
spacetime call space4x place_block -- <draft_id> 0 0 0 '{"commandCore": {}}' 0
spacetime call space4x place_block -- <draft_id> -1 0 0 '{"reactor": {}}' 0
spacetime call space4x commit_design <draft_id> 'Frigate'
```

`spacetime.json` pins the project to the `local` server with `module-path: ./server`.

## Status

The **backend MVP is feature-complete**: the server simulates economy, fleet
movement, ship building, and deterministic combat each tick, and supports
validated block-based ship design — all verified on a local instance. The desktop
client connects, renders Empire + Systems, drives time, and now has a **wgpu 3D
ship editor** (Designer tab: orbit camera, click-to-place blocks, live validity).
What remains is the gameplay order/fleet UI plus CI, tests, and polish. Full spec:
[docs/TDD.md](docs/TDD.md).

## Roadmap & checklist

Mapped to the TDD milestones (§10). ✅ done · 🟡 partial · ⬜ to do.

### M1 — Core Engine ✅
- [x] Cargo workspace with three crates (`shared`, `server`, `client`)
- [x] `shared`: `BlockType` / `OrderType` / `OrderStatus`, block constants, ship-stat formulas, `validate` (11 unit tests)
- [x] All SpacetimeDB tables (§3) + the three enums
- [x] `init` seeds the galaxy with a player and an AI faction (each: starter Scout design + home fleet + ship)
- [x] Economy tick + clock/RNG advance (`advance_ticks` / `advance_days`)
- [ ] Seed scale to ~50 systems + 2 AI factions (TDD target; currently 8 systems + 1 AI)

### M2 — Client Foundation ✅
- [x] Generated Rust client bindings (`spacetime generate`)
- [x] Client connects + subscribes to all tables (`frame_tick` per egui frame)
- [x] Empire Overview (factions, resources, current tick, last `sim_run`)
- [x] System View (systems, owners, planet counts/output)
- [x] Advance Day / Week / Tick buttons (`advance_days` / `advance_ticks`)

### M3 — Ship Editor ✅
- [x] Server: `create_draft` / `place_block` / `remove_block` / `commit_design` (uses shared `validate` + `ship_stats`); verified
- [x] Client: wgpu 3D editor viewport — offscreen render-to-texture, instanced cubes, orbit camera, ghost cube, raycast click-to-place / right-click-remove, R to rotate
- [x] Client: block palette + live stats / validity panel (from shared) + in-editor Commit

### M4 — Simulation Loop 🟡
- [x] `order_build_ship` + BUILD phase (timed build; ship joins fleet)
- [x] `order_move_fleet` + MOVEMENT phase (transit + arrival relocation)
- [x] Deterministic COMBAT phase + `combat_event` log + CLEANUP (remove destroyed ships/membership)
- [x] Verified end-to-end via CLI (build → move → 2v1 combat → destruction)
- [ ] Client: Fleet Manager + order UI + combat-log view

### Cross-cutting (§9)
- [x] Deterministic tick — sorted iteration, seeded PRNG in `GameState`, no wall-clock
- [x] Reducers return `Result<(), String>`; server-side logging
- [x] `u64` auto-inc ids; `Option<u64>` nullable FKs
- [ ] Caller→faction authorization on gameplay reducers (today they reference entities by id; identity binding is a shared-phase concern)
- [ ] Client `env_logger`

### Testing (§12)
- [x] `shared` unit tests — formulas + `validate` (11 passing)
- [x] Manual CLI verification of the full tick pipeline + ship design
- [ ] Automated reducer/integration tests
- [ ] Golden tick tests (would refactor the server-side sim into pure `shared` functions)

### Tooling
- [x] `spacetime` CLI, `wasm32-unknown-unknown` target, build/publish/generate scripts, local hosting
- [ ] CI build gate (fmt, clippy, shared tests, host client build, server wasm build)

### Beyond the original TDD
- [x] Batch time control: `advance_ticks(n)` / `advance_days(d)` (= `d × TICKS_PER_DAY`) + `sim_run` completion log
- [ ] Reconcile the spec narrative (a read-only-client model was trialed, then lifted) and refresh the TDD §3–§8 code snippets to the SpacetimeDB 2.3.0 API

### Not in MVP (future — TDD §13)
Shared persistent universe (scheduled ticks + row-level security), procedural galaxy generation, per-component damage, full 24-orientation rotation, richer AI, diplomacy / logistics / research, client prediction, save slots.
