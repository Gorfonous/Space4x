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
(`rustup target add wasm32-unknown-unknown`), and the SpacetimeDB CLI
(`curl -sSf https://install.spacetimedb.com | sh`).

```sh
# 1. Run the desktop client (native window). It connects to the space4x
#    database at 127.0.0.1:3000 and renders Empire + Systems from live
#    subscriptions; today it drives time via the Advance Day / Advance Tick
#    buttons (order/designer UI is next). Start + publish the server first
#    (steps 2-3) so there's data to show.
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

The **backend MVP is feature-complete**: the server simulates economy, fleet
movement, ship building, and deterministic combat each tick, and supports
validated block-based ship design — all verified on a local instance. The desktop
client connects and renders Empire + Systems and drives time. What remains is
mostly **client UI** (the wgpu 3D ship editor and order/fleet panels) plus CI and
polish. Full spec: [docs/TDD.md](docs/TDD.md).

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

### M3 — Ship Editor 🟡
- [x] Server: `create_draft` / `place_block` / `remove_block` / `commit_design` (uses shared `validate` + `ship_stats`); verified
- [ ] Client: wgpu 3D editor viewport (instanced cubes, orbit camera, ghost cube, click-place / right-remove, rotate)
- [ ] Client: block palette + live stats / validity panel
- [ ] In-editor commit flow

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
