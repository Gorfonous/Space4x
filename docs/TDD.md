# Starframe — Technical Design Document (MVP)

> **Status:** Draft v0.1 · **Audience:** engineers building the MVP · **Companion:** the MVP Game Design Document (GDD)
>
> This TDD translates the GDD and the conceptual schema into a buildable system: a Cargo workspace, real SpacetimeDB Rust tables and reducers, a deterministic tick pipeline, the ship‑editor data flow, and the egui + wgpu client. Where the GDD says *what*, this document says *how*.

---

## 0. Document Meta

| | |
|---|---|
| Working title | **Starframe** (placeholder) |
| Genre | Tick‑based 4X space strategy sandbox |
| Server | SpacetimeDB module, **Rust** |
| Client | **Rust** — `eframe` / `egui` (UI) + `wgpu` (3D ship editor & maps) |
| Multiplayer | **Single‑player now, architected to scale to a shared persistent universe** |
| Delivery | This document lives at `docs/TDD.md` in the project repo |

### 0.1 Relationship to the GDD

The GDD defines the player experience and MVP scope boundaries. This TDD is bound by those boundaries — anything the GDD lists under "NOT in MVP" (physics, real‑time multiplayer, logistics chains, procedural generation, per‑component damage, advanced AI) is out of scope here too, and appears only in §13 (Future Work) as a forward‑looking seam.

### 0.2 How to read this

§1–§2 are the architecture and project skeleton. §3–§7 are the implementation core (data, reducers, tick, ship editor, client). §8–§12 are the contracts, cross‑cutting rules, milestones, and tests that make it shippable. §13–§14 are forward‑looking notes and reference appendices.

---

## 1. Architecture Overview

### 1.1 System context

```text
┌───────────────────────────────────────────────┐        ┌─────────────────────────────────────────┐
│  CLIENT  (native Rust binary, eframe)           │        │  SERVER  (SpacetimeDB module, wasm)        │
│                                                 │        │                                            │
│  ┌───────────────┐   ┌───────────────────────┐ │  WS    │  ┌──────────────┐   ┌──────────────────┐  │
│  │ egui UI        │   │ wgpu ship‑editor 3D   │ │◀──────▶│  │ Tables       │   │ Reducers (the    │  │
│  │ 4 screens      │   │ viewport (instanced)  │ │ subs + │  │ (world state)│   │ only writers)    │  │
│  └──────┬─────────┘   └───────────┬───────────┘ │ calls  │  └──────┬───────┘   └────────┬─────────┘  │
│         │ reads cache   builds from│ draft rows  │        │         │  read/write        │            │
│  ┌──────▼──────────────────────────▼───────────┐│        │  ┌──────▼────────────────────▼─────────┐  │
│  │ Local cache  ← SpacetimeDB SDK subscriptions ││        │  │ Deterministic tick pipeline (§5)     │  │
│  │ reducer calls → ──────────────────────────── ││        │  │ shared sim logic (pure, §6)          │  │
│  └──────────────────────────────────────────────┘│        │  └──────────────────────────────────────┘  │
└───────────────────────────────────────────────┘        └─────────────────────────────────────────┘
                          ▲                                                  ▲
                          └──────────  starframe-shared crate  ──────────────┘
                                  (enums, formulas, pure sim algorithms)
```

**Core principle (from the GDD): the server is the only writer.** The client never mutates world state directly — it issues reducer calls and renders the state that flows back through subscriptions. This is true even in single‑player, because it is the exact contract a shared universe needs; keeping it from day one means scaling out is *additive*, not a rewrite.

### 1.2 The three invariants that make single‑player scale to shared

Everything in this TDD is shaped by three rules. Honor them and the jump to a shared universe is configuration, not surgery.

1. **Identity‑addressed, faction‑scoped state.** Every mutating reducer resolves `ctx.sender` (the caller's `Identity`) to a `Faction` and validates ownership before touching anything. In single‑player there is exactly one human identity, but the code path is identical to many.
2. **The tick is a pure, deterministic function of state + queued orders.** `run_tick(world)` produces the next world with no reads of wall‑clock time, no unseeded randomness, and stable iteration order. This lets us (a) golden‑test it, and (b) later fire it from a *scheduled* reducer on a timer with zero logic changes.
3. **No client authority.** The client may *predict* for responsiveness later, but the server's result is canonical. MVP runs a local instance so latency is ~0 and prediction is unnecessary.

### 1.3 The single‑player ↔ shared seam (concrete)

| Concern | MVP (single‑player) | Shared universe (later) | What changes |
|---|---|---|---|
| Who triggers a tick | Player clicks **Advance Turn** → `advance_tick` reducer | Timer fires `scheduled_tick` reducer | Insert one row into a `tick_timer` scheduled table; tick *logic* unchanged |
| Identities | One human + AI factions | Many humans + AI | Nothing — already identity‑scoped |
| Visibility | Client may subscribe to everything | Row‑level security (RLS) filters per faction | Tighten subscription queries / add RLS rules |
| Order conflicts | None (one human) | Simultaneous orders resolved by tick order | Already handled — orders are queued and resolved deterministically |
| Connection | Local `spacetime start` | Hosted SpacetimeDB | Connection URI only |

> **Design rule:** never write a reducer or query that assumes "only one player." If you find yourself reaching for a global mutable singleton that isn't `GameState`, stop — it almost certainly should be faction‑scoped.

---

## 2. Repository & Workspace Layout

A single Cargo **workspace** with three crates. The `shared` crate is the linchpin: it holds the enums and the *pure* game math, and is compiled into **both** the wasm server module and the native client, so a stat the editor previews can never disagree with the stat the tick resolves.

```text
starframe/
├── Cargo.toml                 # [workspace] members = ["shared","server","client"]
├── README.md
├── rust-toolchain.toml        # pin toolchain
├── docs/
│   └── TDD.md                 # this document
├── shared/                    # crate: starframe-shared  (pure, no DB, no UI)
│   └── src/
│       ├── lib.rs
│       ├── blocks.rs          # BlockType + per-block constants
│       ├── formulas.rs        # mass / cost / hp / thrust / speed / attack / power
│       ├── design.rs          # connectivity + validity (pure functions)
│       └── sim/               # pure tick algorithms over plain structs
│           ├── mod.rs
│           ├── movement.rs
│           ├── combat.rs
│           └── economy.rs
├── server/                    # crate: starframe-module  (cdylib → wasm)
│   └── src/
│       ├── lib.rs             # table defs + reducer entry points
│       ├── tables.rs
│       ├── init.rs            # seed the galaxy
│       └── reducers/
│           ├── mod.rs
│           ├── factions.rs
│           ├── design.rs      # draft edits + commit
│           ├── fleets.rs      # orders: move / attack / build / colonize
│           └── tick.rs        # advance_tick → run_tick(world)
└── client/                    # crate: starframe-client  (bin, eframe)
    └── src/
        ├── main.rs
        ├── app.rs             # eframe::App, screen routing
        ├── connection.rs      # SpacetimeDB SDK connect + subscriptions
        ├── module_bindings/   # GENERATED by `spacetime generate --lang rust`
        ├── state.rs           # cached view models built from row callbacks
        ├── convert.rs         # generated-enum ↔ shared-enum adapters (if needed)
        ├── screens/
        │   ├── empire.rs      # Empire Overview
        │   ├── system.rs      # System View (2D galaxy/system map)
        │   ├── designer.rs    # Ship Designer (hosts wgpu viewport)
        │   └── fleet.rs       # Fleet Manager
        └── render/
            ├── mod.rs
            ├── viewport.rs    # egui_wgpu paint callback
            ├── camera.rs      # orbit camera
            ├── cubes.rs       # instanced cube pipeline
            └── pick.rs        # cursor → grid-cell raycast
```

### 2.1 Crate dependency graph

```text
starframe-shared  ──────┐ (pure logic + SpacetimeType enums)
                        ├──▶ starframe-module  (depends: spacetimedb, shared)
                        └──▶ starframe-client  (depends: spacetimedb_sdk, eframe, egui,
                                                 egui-wgpu, wgpu, glam, shared)
```

### 2.2 Decision: where do the `SpacetimeType` enums live?

`BlockType` and `OrderType` must be one source of truth shared by the editor's math and the database schema. There are two viable patterns:

- **Recommended — enums in `shared`, deriving `SpacetimeType`.** `shared` takes a light dependency on `spacetimedb` for the `SpacetimeType` derive (it builds on the same `spacetimedb_sats` serialization layer the client SDK already uses, so it does not bloat the client). The server uses these types directly in `#[table]`/reducer signatures. The client's generated `module_bindings` reproduce structurally identical enums; `client/src/convert.rs` provides trivial `From` impls if Rust's nominal typing requires bridging generated ↔ shared.
- **Fallback — enums in `server`, mirrored as a `repr(u8)` in `shared`.** If the `shared`→`spacetimedb` dependency ever proves awkward, define the `SpacetimeType` enums in `server` and a plain `#[repr(u8)]` mirror in `shared`, with a `u8` conversion. Costs one small duplicated enum.

Start with the recommended pattern; the fallback is a localized change if needed.

### 2.3 Tooling

- **SpacetimeDB CLI** (`spacetime`): `spacetime start` (local), `spacetime publish starframe` (deploy module), `spacetime generate --lang rust --out-dir client/src/module_bindings` (regenerate client bindings after schema/reducer changes), `spacetime logs starframe`, `spacetime sql` (ad‑hoc inspection).
- **Build:** `cargo build -p starframe-shared`, server published via `spacetime publish`, `cargo run -p starframe-client`.
- **CI:** `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, `cargo test -p starframe-shared` (the pure logic — fast, deterministic), and a build of the wasm module target.
- **Pin versions** in each `Cargo.toml` (e.g. `spacetimedb = "=1.x"`, matching `spacetimedb_sdk`) — SpacetimeDB's macro/codegen surface evolves; mismatched module/SDK versions break generation.

---

## 3. Data Model — SpacetimeDB Tables

This is the refinement of the conceptual schema into real SpacetimeDB Rust. Conventions:

- `id: u64` primary key with `#[auto_inc]` everywhere; **IDs are never reused**.
- `#[index(btree)]` on every foreign‑key column we filter by (`system_id`, `faction_id`, `design_id`, …).
- `#[unique]` where a natural key must be enforced by the DB (e.g. `faction.name`).
- **Public vs private:** `public` tables are subscribable by clients. In MVP all are `public` for convenience; §8.3 notes which become RLS‑filtered in the shared phase. `player_account` is the one to lock down first.
- `Option<u64>` foreign keys use `0`‑free semantics — `None` means "unowned/none", never `Some(0)`.

```rust
use spacetimedb::{table, reducer, ReducerContext, Table, Identity, Timestamp, SpacetimeType};

// ── 3.1 Singletons & accounts ─────────────────────────────────────────────

/// Single-row table (id is always 1). Holds the simulation clock + RNG seed.
#[table(name = game_state, public)]
pub struct GameState {
    #[primary_key]
    pub id: u64,            // always 1
    pub current_tick: u64,
    pub rng_seed: u64,      // advanced each tick; reserved for future stochastic rules
    pub schema_version: u32,
}

/// Maps a connection Identity to the faction it controls.
/// PRIVATE in the shared phase (RLS: a player sees only their own row).
#[table(name = player_account, public)]
pub struct PlayerAccount {
    #[primary_key]
    pub identity: Identity,
    #[unique]
    pub faction_id: u64,
    pub created_at: Timestamp,
}

// ── 3.2 Empire & galaxy ───────────────────────────────────────────────────

#[table(name = faction, public)]
pub struct Faction {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[unique]
    pub name: String,
    pub is_ai: bool,
    pub minerals: i64,
    pub energy: i64,
    pub research: i64,
    pub home_system_id: u64,
}

#[table(name = star_system, public)]
pub struct StarSystem {
    #[primary_key] #[auto_inc]
    pub id: u64,
    pub name: String,
    pub owner_faction_id: Option<u64>,
    pub x: f32, pub y: f32, pub z: f32,   // galaxy-space position (light-years, abstract)
}

#[table(name = planet, public)]
pub struct Planet {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub owner_faction_id: Option<u64>,
    pub population: i64,
    pub minerals_output: i64,
    pub energy_output: i64,
    pub research_output: i64,
}

// ── 3.3 Ship designs (the editor's committed output) ──────────────────────

/// Immutable once committed. Stats are precomputed at commit time.
#[table(name = ship_design, public)]
pub struct ShipDesign {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    pub name: String,
    pub total_mass: f32,
    pub total_cost: i64,
    pub max_hp: i64,
    pub thrust: f32,
    pub attack: i64,
    pub block_count: i32,
    pub created_tick: u64,
}

#[table(name = ship_design_block, public)]
pub struct ShipDesignBlock {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub design_id: u64,
    pub x: i32, pub y: i32, pub z: i32,
    pub block_type: BlockType,
    pub rotation: u8,        // MVP: 0..4 (yaw, 90° steps). 24-orientation is future work.
}

// ── 3.4 Runtime ships & fleets ────────────────────────────────────────────

#[table(name = ship, public)]
pub struct Ship {
    #[primary_key] #[auto_inc]
    pub id: u64,
    pub design_id: u64,
    #[index(btree)]
    pub faction_id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub x: f32, pub y: f32, pub z: f32,    // in-system position (abstract)
    pub hp: i64,
    pub fuel: i64,
}

#[table(name = fleet, public)]
pub struct Fleet {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub name: String,
    // transit state (extends the conceptual schema; see §5.4)
    pub dest_system_id: Option<u64>,
    pub arrival_tick: Option<u64>,
}

/// Join table. A ship is in at most one fleet → ship_id is the primary key.
#[table(name = fleet_ship, public)]
pub struct FleetShip {
    #[primary_key]
    pub ship_id: u64,
    #[index(btree)]
    pub fleet_id: u64,
}

// ── 3.5 Orders (tick-resolved commands) ───────────────────────────────────

#[table(name = order, public)]
pub struct Order {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    pub order_type: OrderType,
    pub status: OrderStatus,
    pub target_id: Option<u64>,
    pub target_system_id: Option<u64>,
    pub ship_id: Option<u64>,
    pub fleet_id: Option<u64>,
    pub created_tick: u64,
    pub complete_tick: Option<u64>,   // for timed orders (e.g. ship build)
}

// ── 3.6 Editor drafts (live, mutable client edits before commit) ──────────

#[table(name = ship_design_draft, public)]
pub struct ShipDesignDraft {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    pub name: String,
    pub updated_at: Timestamp,
}

#[table(name = ship_design_draft_block, public)]
pub struct ShipDesignDraftBlock {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub draft_id: u64,
    pub x: i32, pub y: i32, pub z: i32,
    pub block_type: BlockType,
    pub rotation: u8,
}

// ── 3.7 Combat log ────────────────────────────────────────────────────────

#[table(name = combat_event, public)]
pub struct CombatEvent {
    #[primary_key] #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub tick: u64,
    pub system_id: u64,
    pub attacker_ship_id: u64,
    pub defender_ship_id: u64,
    pub attacker_faction_id: u64,
    pub defender_faction_id: u64,
    pub damage_dealt: i64,
    pub destroyed: bool,
}

// ── 3.8 Scheduling seam (UNUSED in MVP; flip on for the shared phase) ──────

/// When we move to a timed shared universe, insert one row here and the
/// scheduler invokes `scheduled_tick`. In MVP this table stays empty and the
/// player drives ticks via the `advance_tick` reducer.
#[table(name = tick_timer, scheduled(scheduled_tick))]
pub struct TickTimer {
    #[primary_key] #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: spacetimedb::ScheduleAt,
}
```

### 3.9 Enums

`CommandCore` is added to the GDD's block list to satisfy the "must have ≥1 command core" rule. `OrderStatus` is added so the tick can mark progress idempotently.

```rust
#[derive(SpacetimeType, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockType {
    Hull,
    Engine,
    Weapon,
    Reactor,
    Sensor,
    CommandCore,
}

#[derive(SpacetimeType, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderType {
    MoveFleet,
    Attack,
    BuildShip,
    Colonize,
    // Note: DesignShipCommit from the conceptual schema is NOT an order — design
    // commit is an immediate reducer (the editor needs instant feedback). See §4.
}

#[derive(SpacetimeType, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderStatus { Pending, Active, Done, Failed }
```

### 3.10 Access‑pattern → index map

| Query (per tick / per screen) | Index used |
|---|---|
| Ships in a system (combat, system view) | `ship.system_id` |
| Ships / designs / fleets / orders owned by a faction | `*.faction_id` |
| Blocks of a design (instancing, stat calc) | `ship_design_block.design_id` |
| Draft blocks being edited | `ship_design_draft_block.draft_id` |
| Planets in a system (economy, system view) | `planet.system_id` |
| Combat events for a tick (combat log) | `combat_event.tick` |
| Ship → fleet membership | `fleet_ship` PK on `ship_id`, index on `fleet_id` |

---

## 4. Reducers — the Server API

Reducers are the **only** way state changes. Every mutating reducer follows the same skeleton: resolve caller → validate ownership/preconditions → mutate → (optionally) log. They return `Result<(), String>`; the `Err` string surfaces to the client via the SDK's reducer‑status callback (§8.2).

> **Client contract (current phase).** The client is **read‑only** except for a **single command: advance the simulation by N ticks** — `advance_ticks(n)`, or `advance_days(d)` = `d × TICKS_PER_DAY`. The server runs the batch in one transaction, appends a `sim_run` row as the completion signal, and then all state is read‑only via subscriptions. **Every other reducer below (`create_faction`, the `order_*` family, the ship‑editor `*_draft` / `commit_design`) is NOT in the client contract yet** — the starting world (including the player's faction) is **seeded server‑side in `init`**, and those reducers exist for tests/admin and become *future* client messages. ("One mutation = advance time" is also the exact shape the shared‑universe phase wants, where a scheduled tick replaces the client's call.)

### 4.1 Lifecycle

```rust
#[reducer(init)]
pub fn init(ctx: &ReducerContext) {
    // Insert GameState{ id:1, current_tick:0, rng_seed: 0x9E3779B97F4A7C15, schema_version:1 }
    // Seed the galaxy (see §10 M1): star systems, planets, and — because the
    // client is read-only and cannot create one — the PLAYER's faction plus an
    // AI faction, each with a home system.
}

#[reducer(client_connected)]
pub fn on_connect(ctx: &ReducerContext) {
    // If ctx.sender has no PlayerAccount yet, leave it — faction creation is explicit.
}

#[reducer(client_disconnected)]
pub fn on_disconnect(ctx: &ReducerContext) { /* no-op in MVP */ }
```

### 4.2 Helper: resolve caller → faction (used by every gameplay reducer)

```rust
fn caller_faction(ctx: &ReducerContext) -> Result<Faction, String> {
    let acct = ctx.db.player_account().identity().find(ctx.sender)
        .ok_or("no faction for this identity — call create_faction first")?;
    ctx.db.faction().id().find(acct.faction_id)
        .ok_or("faction missing".into())
}
```

### 4.3 Faction & account

```rust
#[reducer]
pub fn create_faction(ctx: &ReducerContext, name: String) -> Result<(), String> {
    if ctx.db.player_account().identity().find(ctx.sender).is_some() {
        return Err("identity already bound to a faction".into());
    }
    // pick a home system, insert Faction (auto_inc id), bind PlayerAccount.
    Ok(())
}
```

### 4.4 Ship editor (draft lifecycle + atomic commit)

Design commit is an **immediate reducer**, not a tick order — the editor must give instant feedback and a committed design has no in‑world side effects until a ship is built from it.

```rust
#[reducer]
pub fn create_draft(ctx: &ReducerContext, name: String) -> Result<(), String>;

/// Place or overwrite the block at (x,y,z) in a draft the caller owns.
#[reducer]
pub fn place_block(ctx: &ReducerContext, draft_id: u64,
                   x: i32, y: i32, z: i32,
                   block_type: BlockType, rotation: u8) -> Result<(), String> {
    let f = caller_faction(ctx)?;
    let draft = ctx.db.ship_design_draft().id().find(draft_id)
        .ok_or("draft not found")?;
    if draft.faction_id != f.id { return Err("not your draft".into()); }
    // delete any existing draft block at (x,y,z), then insert the new one.
    Ok(())
}

#[reducer]
pub fn remove_block(ctx: &ReducerContext, draft_id: u64, x: i32, y: i32, z: i32)
    -> Result<(), String>;

/// Validate (connectivity + required blocks + power), compute stats, and write
/// an immutable ShipDesign + its ShipDesignBlocks. Leaves the draft intact so
/// the player can keep iterating.
#[reducer]
pub fn commit_design(ctx: &ReducerContext, draft_id: u64, name: String)
    -> Result<(), String> {
    let f = caller_faction(ctx)?;
    let blocks = collect_draft_blocks(ctx, draft_id, f.id)?;       // Vec<BlockPlacement>
    let report = starframe_shared::design::validate(&blocks);      // pure (§6)
    if !report.is_valid { return Err(report.problems.join("; ")); }
    let stats = starframe_shared::formulas::ship_stats(&blocks);   // pure (§6)
    // insert ShipDesign{ stats, block_count, created_tick }, then one
    // ShipDesignBlock per placement. All within this reducer = one atomic txn.
    Ok(())
}
```

> **Why drafts exist:** the editor mutates `ship_design_draft_block` rows live (every block placement is a reducer call that echoes back through the subscription). `commit_design` snapshots the validated draft into the immutable `ship_design` tables in a single transaction. Designs are never edited in place — iterate on the draft, commit a new design.

### 4.5 Orders (queued, resolved by the tick)

```rust
#[reducer]
pub fn order_move_fleet(ctx: &ReducerContext, fleet_id: u64, dest_system_id: u64)
    -> Result<(), String>;   // validates ownership + that dest exists; queues Order

#[reducer]
pub fn order_attack(ctx: &ReducerContext, fleet_id: u64, target_system_id: u64)
    -> Result<(), String>;   // MVP: "attack-move" — combat auto-resolves on arrival

#[reducer]
pub fn order_build_ship(ctx: &ReducerContext, design_id: u64, system_id: u64)
    -> Result<(), String> {
    // validate design owned by caller; check faction can afford total_cost;
    // DEDUCT cost now; queue Order{BuildShip, complete_tick = now + build_ticks(design)}.
}

#[reducer]
pub fn cancel_order(ctx: &ReducerContext, order_id: u64) -> Result<(), String>;
```

### 4.6 The tick entry points — the client's one command

The tick *logic* lives in one place (`run_tick`, §5). The client never calls it directly; it asks the server to process a **batch** of ticks. `advance_ticks(n)` is the primitive; `advance_days(d)` = `d × TICKS_PER_DAY` is the convenience ("go forward one day"). The batch runs in **one transaction** and appends a `sim_run` row as the completion signal, after which all state is read‑only via subscriptions. The shared phase adds a timer that calls the same `run_tick`.

```rust
const MAX_TICKS_PER_CALL: u64 = 100_000;   // bound one transaction's work

/// THE client→server command: process `num_ticks` ticks, then record a sim_run.
#[reducer]
pub fn advance_ticks(ctx: &ReducerContext, num_ticks: u64) -> Result<(), String> { do_advance(ctx, num_ticks) }

/// Convenience: days × TICKS_PER_DAY ticks.
#[reducer]
pub fn advance_days(ctx: &ReducerContext, days: u64) -> Result<(), String> {
    do_advance(ctx, days * starframe_shared::TICKS_PER_DAY)
}

/// Shared phase: insert a TickTimer row → one tick per fire. Same `run_tick`.
#[reducer]
pub fn scheduled_tick(ctx: &ReducerContext, _t: TickTimer) -> Result<(), String> { run_tick(ctx) }

fn do_advance(ctx: &ReducerContext, n: u64) -> Result<(), String> {
    // validate 1..=MAX_TICKS_PER_CALL; loop run_tick n times; insert
    // SimRun { requested_ticks, from_tick, to_tick, completed_at }. One txn.
    Ok(())
}
```

The completion signal is the `sim_run` table (`run_id`, `requested_ticks`, `from_tick`, `to_tick`, `completed_at`): the client subscribes to it, and the new row (plus the reducer‑status callback) tells it the batch is done.

### 4.7 Reducer catalogue (summary)

**In the client contract today: only the advance commands.** Everything else is server‑internal or seeded and becomes a *future* client message.

| Reducer | Client contract? | Validates | Effect |
|---|---|---|---|
| `advance_ticks` / `advance_days` | **YES — the only one** | `1..=MAX_TICKS_PER_CALL` | run N ticks (§5), append `sim_run` |
| `init` | lifecycle | — | seed galaxy + player & AI factions + `GameState` |
| `scheduled_tick` | shared‑phase only | — | one tick per timer fire |
| `create_faction` | deferred | identity unbound | new `Faction` (+ `PlayerAccount`) |
| `create_draft` / `place_block` / `remove_block` | deferred | draft ownership | mutate draft tables |
| `commit_design` | deferred | connectivity, required blocks, power, ownership | immutable `ShipDesign` + blocks |
| `order_move_fleet` / `order_attack` | deferred | fleet ownership, dest exists | queue `Order` |
| `order_build_ship` | deferred | design ownership, affordability | deduct cost, queue timed `Order` |
| `cancel_order` | deferred | order ownership, still `Pending` | mark `Failed`/delete; refund build cost |

---

## 5. Tick Simulation Pipeline

The heart of the game. `run_tick` executes fixed, ordered phases. **Determinism is non‑negotiable** (it's what lets us golden‑test and later schedule it).

### 5.1 Determinism rules

1. **Stable iteration order.** Never iterate a table and depend on row order. Collect into a `Vec`, **sort by `id`**, then process. (SpacetimeDB iteration order is not a contract; `HashMap` order is hostile.)
2. **No wall‑clock.** Resolution never reads `ctx.timestamp`. Game time is `GameState.current_tick` only.
3. **Seeded randomness only.** MVP combat is fully deterministic (no RNG). Any future stochastic rule must draw from a PRNG seeded by `GameState.rng_seed`, which is advanced and persisted each tick.
4. **Pure core.** Phase algorithms live in `starframe-shared::sim` as functions over plain structs (no DB handle). The reducer loads rows → builds plain inputs → calls pure functions → writes results back. This is the seam that makes the tick unit‑testable (§12).

### 5.2 Phase order

```text
run_tick(ctx):
  1. INGEST   — load GameState; mark Pending orders Active
  2. MOVEMENT — advance fleets in transit; complete arrivals
  3. COMBAT   — resolve every system containing ≥2 hostile factions
  4. ECONOMY  — add each planet's output to its owner faction
  5. BUILD    — complete BuildShip orders whose complete_tick has arrived
  6. CLEANUP  — delete destroyed ships + their fleet_ship rows; mark orders Done
  7. ADVANCE  — current_tick += 1; rng_seed = splitmix64(rng_seed); persist GameState
```

Order matters: movement before combat (arriving fleets fight this tick), combat before economy (destroyed planets/owners settle), build last (new ships don't act until next tick).

### 5.3 Pseudocode (per phase)

```rust
fn run_tick(ctx: &ReducerContext) -> Result<(), String> {
    let mut gs = ctx.db.game_state().id().find(1).ok_or("no GameState")?;
    let now = gs.current_tick;

    // 2. MOVEMENT ----------------------------------------------------------
    let mut fleets: Vec<Fleet> = ctx.db.fleet().iter().collect();
    fleets.sort_by_key(|f| f.id);
    for mut f in fleets {
        if let (Some(dest), Some(arr)) = (f.dest_system_id, f.arrival_tick) {
            if now + 1 >= arr {                       // arrives this tick
                relocate_fleet_and_ships(ctx, &f, dest);
                f.dest_system_id = None; f.arrival_tick = None;
                ctx.db.fleet().id().update(f);
            }
        }
    }

    // 3. COMBAT ------------------------------------------------------------
    for sys_id in systems_with_conflict(ctx) {        // sorted Vec<u64>
        let mut ships = ships_in_system(ctx, sys_id);  // sorted by id, with cached `attack`
        let events = starframe_shared::sim::combat::resolve(&mut ships); // pure
        for s in &ships { ctx.db.ship().id().update(s.to_row()); }
        for e in events { ctx.db.combat_event().insert(e.into_row(now, sys_id)); }
    }

    // 4. ECONOMY -----------------------------------------------------------
    let mut planets: Vec<Planet> = ctx.db.planet().iter().collect();
    planets.sort_by_key(|p| p.id);
    for p in planets {
        if let Some(owner) = p.owner_faction_id {
            if let Some(mut fac) = ctx.db.faction().id().find(owner) {
                fac.minerals += p.minerals_output;
                fac.energy   += p.energy_output;
                fac.research += p.research_output;
                ctx.db.faction().id().update(fac);
            }
        }
    }

    // 5. BUILD -------------------------------------------------------------
    let mut orders: Vec<Order> = ctx.db.order().iter()
        .filter(|o| o.order_type == OrderType::BuildShip && o.status == OrderStatus::Active)
        .collect();
    orders.sort_by_key(|o| o.id);
    for mut o in orders {
        if o.complete_tick.map_or(false, |c| now + 1 >= c) {
            spawn_ship_from_design(ctx, &o)?;          // new Ship at target system
            o.status = OrderStatus::Done;
            ctx.db.order().id().update(o);
        }
    }

    // 6. CLEANUP -----------------------------------------------------------
    remove_destroyed_ships(ctx);                       // hp <= 0 → delete ship + fleet_ship

    // 7. ADVANCE -----------------------------------------------------------
    gs.current_tick += 1;
    gs.rng_seed = splitmix64(gs.rng_seed);
    ctx.db.game_state().id().update(gs);
    Ok(())
}
```

### 5.4 Movement model

Travel is between systems, abstractly (no in‑system physics). On `order_move_fleet`:

```text
speed       = min(ship.speed for ship in fleet)         // see §6 formulas; slowest ship sets pace
distance    = euclidean(src.xyz, dest.xyz)
travel_ticks= max(1, ceil(distance / (speed * SPEED_SCALE)))
fleet.dest_system_id = dest;  fleet.arrival_tick = current_tick + travel_ticks
```

Each tick the MOVEMENT phase checks for arrivals and relocates the fleet **and all member ships** to the destination system (and sets their in‑system coordinates). Fuel is decremented per jump (MVP: flat cost; out‑of‑fuel handling is a future refinement).

### 5.5 Combat model (deterministic, no RNG)

A system has "conflict" when it contains living ships of ≥2 distinct factions (MVP: all factions are mutually hostile — no diplomacy). Resolution (`shared::sim::combat::resolve`):

```text
ships sorted by id
for attacker in ships (in id order):
    if attacker.attack == 0 or attacker.hp <= 0: continue
    target = living enemy ship with the lowest id           // stable, deterministic
    if no target: continue
    dmg = attacker.attack                                    // = Σ weapon block damage
    target.hp -= dmg
    emit CombatEvent{ attacker, target, dmg, destroyed: target.hp <= 0 }
```

One round per tick — battles play out over several ticks, which suits a turn‑based feel and gives the combat log something to show. Destroyed ships are removed in CLEANUP (not mid‑loop) so every attacker resolves against a stable snapshot.

### 5.6 Economy model

Each planet contributes its `minerals_output` / `energy_output` / `research_output` to its owner faction every tick. MVP has no upkeep or logistics. `Colonize` orders set `planet.owner_faction_id` (and possibly `star_system.owner_faction_id`) on resolution.

### 5.7 Build model

`order_build_ship` deducts cost immediately and queues a timed order. `build_ticks(design) = max(1, ceil(design.total_cost / BUILD_RATE))`. The BUILD phase spawns the `Ship` (full hp/fuel from design stats) at the target system when `complete_tick` arrives. Cancelling a still‑`Pending` build refunds the cost.

---

## 6. Ship Design System (core feature deep‑dive)

All math here is **pure and lives in `starframe-shared`**, so the editor's live preview and the server's `commit_design` compute identical numbers.

### 6.1 Grid & rotation model

- A design is a set of integer cells `(x, y, z)` each holding one `BlockType` + `rotation`.
- `rotation: u8` is **0..4** (yaw in 90° steps) for MVP — enough for engines/weapons to face. Full 24‑orientation is future work (§13).
- No two blocks share a cell (enforced by `place_block` overwrite semantics + a commit‑time check).

### 6.2 Per‑block constants (`shared/src/blocks.rs`)

| Block | mass | cost | hp | thrust | attack | power |
|---|---:|---:|---:|---:|---:|---:|
| Hull | 1.0 | 10 | 50 | 0 | 0 | 0 |
| Engine | 2.0 | 25 | 20 | 10.0 | 0 | −5 |
| Weapon | 1.5 | 40 | 20 | 0 | 10 | −8 |
| Reactor | 3.0 | 50 | 30 | 0 | 0 | **+20** |
| Sensor | 0.5 | 20 | 10 | 0 | 0 | −2 |
| CommandCore | 2.0 | 100 | 100 | 0 | 0 | −3 |

(Constants are first‑pass balance knobs — tune freely; they live in one file.)

### 6.3 Derived stats (`shared/src/formulas.rs`)

```text
mass   = Σ block.mass
cost   = Σ block.cost
max_hp = Σ block.hp
thrust = Σ engine.thrust
attack = Σ weapon.attack
power  = Σ block.power           // reactors positive, consumers negative
speed  = clamp(thrust / mass, 0, SPEED_MAX)   // galaxy units per (tick * SPEED_SCALE)
```

```rust
pub struct ShipStats {
    pub mass: f32, pub cost: i64, pub max_hp: i64,
    pub thrust: f32, pub attack: i64, pub power: i64, pub speed: f32,
}
pub fn ship_stats(blocks: &[BlockPlacement]) -> ShipStats { /* sums + speed */ }
```

### 6.4 Validity (`shared/src/design.rs`)

A design is valid iff **all** hold:

1. **Connected** — all cells form one component under 6‑neighbour adjacency (flood fill from any `CommandCore`; every cell must be reached).
2. **Has a command core** — `command_core_count >= 1`.
3. **Can move** — `engine_count >= 1` (a stationary ship is useless in MVP).
4. **Power‑balanced** — `power >= 0` (reactors meet or exceed consumption).

```rust
pub struct ValidationReport { pub is_valid: bool, pub problems: Vec<String> }

pub fn validate(blocks: &[BlockPlacement]) -> ValidationReport {
    // 1. empty? -> invalid
    // 2. count command cores / engines; sum power
    // 3. flood-fill connectivity from first command core over 6-neighbours
    // collect human-readable problems for the editor to display
}
```

The same `validate` runs **client‑side every edit** (instant red/green feedback in the stats panel) and **server‑side in `commit_design`** (authority). They cannot disagree — it's the same function.

### 6.5 Draft → Design commit flow

```text
[client] place/remove block ──reducer──▶ ship_design_draft_block rows mutate
                                              │ subscription echo
                                              ▼
[client] rebuild cube mesh + run shared::validate + shared::ship_stats → stats panel
                                              │
[player clicks Commit] ──commit_design──▶ [server] validate (authority) + ship_stats
                                              │ atomic txn
                                              ▼
                         immutable ShipDesign + ShipDesignBlock rows
```

---

## 7. Client Architecture (egui + wgpu)

### 7.1 Host & app shell

`eframe` hosts the window and the egui+wgpu render loop. `StarframeApp` implements `eframe::App`; `update()` runs each frame, routes to the active screen, and repaints from the local cache.

```rust
pub struct StarframeApp {
    conn:   StdbConnection,     // SpacetimeDB SDK handle (connection.rs)
    cache:  WorldCache,         // view models built from row callbacks (state.rs)
    screen: Screen,             // Empire | System | Designer | Fleet
    editor: EditorState,        // active draft id, camera, hovered cell, selected block
}

enum Screen { Empire, System { id: u64 }, Designer, Fleet }

impl eframe::App for StarframeApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.conn.pump();                       // drain SDK callbacks → update cache
        egui::TopBottomPanel::top("nav").show(ctx, |ui| self.nav_bar(ui));
        egui::CentralPanel::default().show(ctx, |ui| match self.screen {
            Screen::Empire        => screens::empire::show(ui, &self.cache, &self.conn),
            Screen::System { id } => screens::system::show(ui, &self.cache, id),
            Screen::Designer      => screens::designer::show(ui, frame, &mut self.editor, &self.conn),
            Screen::Fleet         => screens::fleet::show(ui, &self.cache, &self.conn),
        });
        ctx.request_repaint();                  // keep the 3D viewport live
    }
}
```

### 7.2 Connection, subscriptions & cache

The SDK connects to the local SpacetimeDB instance, registers a subscription set, and fires row callbacks (`on_insert` / `on_update` / `on_delete`). Each callback updates `WorldCache`; screens render purely from the cache.

```rust
// connection.rs — on connect, register the MVP subscription set:
conn.subscription_builder()
    .on_applied(|_| log::info!("subscriptions applied"))
    .subscribe([
        "SELECT * FROM game_state",
        "SELECT * FROM star_system",
        "SELECT * FROM planet",
        "SELECT * FROM faction",
        "SELECT * FROM ship",
        "SELECT * FROM fleet",
        "SELECT * FROM fleet_ship",
        "SELECT * FROM ship_design",
        "SELECT * FROM ship_design_block",
        "SELECT * FROM ship_design_draft",
        "SELECT * FROM ship_design_draft_block",
        "SELECT * FROM order",
        "SELECT * FROM combat_event",
    ]);
// Shared phase: replace `*` with faction-scoped WHERE clauses / RLS (§8.3).
```

```rust
// state.rs — row callbacks feed the cache (example)
conn.db.faction().on_insert(|_ctx, row| cache_upsert_faction(row));
conn.db.faction().on_update(|_ctx, _old, new| cache_upsert_faction(new));
conn.db.ship().on_delete(|_ctx, row| cache_remove_ship(row.id));
```

### 7.3 The four screens

| Screen | Reads | Actions (reducer calls) |
|---|---|---|
| **Empire Overview** | factions, resources, ship/fleet/design counts; `game_state.current_tick`; **Advance Turn** | `advance_tick` |
| **System View** | a system's planets, ships, owners; 2D galaxy map (egui `Painter`) | select system; `order_move_fleet`, `order_build_ship` |
| **Ship Designer** | active draft blocks → 3D viewport + live stats | `create_draft`, `place_block`, `remove_block`, `commit_design` |
| **Fleet Manager** | fleets, membership, transit/arrival status | `order_move_fleet`, `order_attack`, regroup |

MVP renders the galaxy/system maps in **2D** with egui's `Painter` (nodes + links) — cheap and readable. The **3D `wgpu` viewport is reserved for the ship editor**, where it earns its cost.

### 7.4 Ship‑editor 3D viewport (`render/`)

The viewport is an egui region backed by a `wgpu` paint callback (`egui_wgpu::CallbackTrait`). eframe exposes the shared `wgpu` device/queue via `CreationContext::wgpu_render_state`, so the editor renders into the same surface as the UI.

```text
designer::show(ui, frame, editor, conn):
  ├── left panel: block palette (select BlockType), stats panel (shared::ship_stats + validate)
  └── central viewport:
        allocate a rect → egui::PaintCallback(egui_wgpu::Callback::new(rect, ViewportPaint{..}))
        ViewportPaint renders:
          • ground grid + axis gizmo
          • one instanced cube per draft block (color by BlockType)
          • a translucent "ghost" cube at the hovered cell
        input (handled in egui, before paint):
          • drag       → orbit camera (camera.rs)
          • scroll     → zoom
          • hover      → pick.rs: cursor ray → target cell (face-adjacent to hovered cube,
                          or grid-plane intersection when pointing at empty space)
          • left-click → place_block(draft, cell, selected_type, rotation)
          • right-click→ remove_block(draft, cell)
          • R          → rotation = (rotation + 1) % 4
```

**Rendering** (`cubes.rs`): one unit‑cube vertex/index buffer + a per‑instance buffer (`model matrix`, `color`); a single instanced draw call. Camera (`camera.rs`): orbit (yaw/pitch/radius) → view‑projection uniform. **Picking** (`pick.rs`): unproject the cursor with the inverse view‑proj to form a world ray; if it hits a placed cube, the new cell is the neighbour across the hit face; else intersect the ground plane and snap to the grid cell.

**Edit echo:** placement calls a reducer; the draft‑block subscription echoes the change back and the mesh rebuilds from the cache — one source of truth. (A local‑first optimistic path is an optional later refinement; on a local instance the round‑trip is already imperceptible.)

---

## 8. Client–Server Sync Model

### 8.1 The client is read‑only; one command advances time

The client holds no authoritative state — it subscribes, mirrors rows into `WorldCache`, and renders. In the current phase it issues **exactly one** mutation: `advance_ticks(n)` / `advance_days(d)` (= `d × TICKS_PER_DAY`). The server processes the batch in one transaction, appends a `sim_run` row, and streams the resulting rows back; the client then reads the new state. There is no other client→server write yet — faction setup, orders, and design commits are seeded/internal for now and arrive as future commands (§4 note).

### 8.2 The completion signal

A batch is one atomic transaction, so the client sees the final world state and the new `sim_run` row together. "Done" is signalled two ways: (1) the SDK's reducer‑status callback when `advance_*` commits — a failed status carries the `Err(String)`, e.g. "num_ticks must be at least 1"; and (2) a new `sim_run` row (`run_id`, `requested_ticks`, `from_tick`, `to_tick`, `completed_at`) appearing in the subscription. The client refreshes its views off either.

### 8.3 What changes for the shared universe

- **RLS / scoped subscriptions:** replace `SELECT *` with faction‑scoped queries (your ships/fleets/designs/orders) plus the public galaxy (`star_system`, `planet`). `player_account` becomes private (each identity sees only its own row).
- **Tick trigger:** insert a `tick_timer` row → `scheduled_tick` fires on a cadence; the client's `advance_*` command is removed (or gated to admins) since time then advances on its own.
- **Order conflict:** already handled — orders queue and resolve in deterministic id order during the tick.
- **Prediction/reconciliation:** optional, only if hosted latency hurts the editor; the authority model already supports it.

---

## 9. Cross‑Cutting Concerns

- **Authority & validation:** every mutating reducer resolves `ctx.sender` → faction and checks ownership of any referenced ship/fleet/design/order before acting. Never trust client‑supplied ids without an ownership check. (True even in single‑player — it's the shared‑phase contract.)
- **IDs:** `u64` `#[auto_inc]`, never reused; `Option<u64>` for nullable FKs, `None` = none.
- **Determinism (recap):** sorted iteration, no wall‑clock in resolution, seeded PRNG persisted in `GameState` (§5.1). This is the single most important reliability property.
- **Logging:** server uses SpacetimeDB's `log` (`spacetime logs starframe`); client uses `env_logger`. Log every tick boundary with `current_tick` for traceability.
- **Schema migration:** SpacetimeDB auto‑migrates compatible changes (additive columns/tables); destructive changes in dev use `spacetime publish --clear-database`. Bump `GameState.schema_version` on breaking changes and keep a short migration note in `docs/`.
- **Error policy:** reducers return `Result<(), String>`; never panic in a reducer (a panic aborts the txn opaquely). Validate and return a descriptive `Err`.
- **Time vs ticks:** gameplay never depends on real time. `Timestamp` is used only for bookkeeping (`updated_at`, `created_at`), never in resolution.

---

## 10. Build & Milestone Plan

Maps the GDD's four phases to technical milestones. Each is independently demoable with explicit acceptance criteria.

### M1 — Core Engine (GDD Phase 1)
**Deliver:** workspace + 3 crates; `shared` enums + constants + formulas + `validate`; **all** tables (§3); `init` seeds ~50 systems, planets, 1 player + 2 AI factions; `create_faction`; `advance_tick` running ECONOMY + ADVANCE only.
**Acceptance:** `spacetime publish`; `spacetime call <db> advance_days 1` (= `TICKS_PER_DAY` ticks); `spacetime sql "SELECT * FROM faction"` shows resources risen and a `sim_run` row recorded. `cargo test -p starframe-shared` green.
**Tests:** unit tests for all formulas and `validate` fixtures.

### M2 — Client Foundation (GDD Phase 2)
**Deliver:** `spacetime generate` bindings; client connects + subscribes; **Empire Overview** and **System View** render live seeded data; an **Advance Day** button calls `advance_days(1)` (the client's only command).
**Acceptance:** launch client, see the seeded galaxy and faction resources, click **Advance Day**, watch `current_tick` jump by `TICKS_PER_DAY` and resources update live (no restart).
**Tests:** manual smoke — connect, observe a row callback updating the UI.

### M3 — Ship Editor (GDD Phase 3)
**Deliver:** draft tables + `create_draft`/`place_block`/`remove_block`/`commit_design` (with `validate` + `ship_stats`); wgpu viewport with orbit camera, instanced cubes, ghost cube, click‑place/right‑remove, rotate; live stats + validity panel.
**Acceptance:** in the editor, build a connected ship with a command core + engine + reactor, see green/valid + correct mass/cost/hp/power, click **Commit**, and confirm a `ship_design` row with matching precomputed stats.
**Tests:** connectivity edge cases (detached block → invalid), power‑balance boundary, commit writes N blocks.

### M4 — Simulation Loop (GDD Phase 4)
**Deliver:** `order_build_ship` + BUILD phase; fleet create + `order_move_fleet` (transit + arrival relocation); COMBAT + `combat_event`; **Fleet Manager** + combat‑log UI.
**Acceptance (the GDD success criteria):** create a faction → design a ship → build it → form a fleet → move it between systems over several ticks → engage an enemy fleet → watch HP drop and ships get destroyed **deterministically** (same seed + orders → identical outcome). Advance time (`advance_days`) and watch the universe change.
**Tests:** golden tick tests (§12) for movement arrival, a scripted battle, and an economy delta.

---

## 11. (folded into §10 acceptance criteria)

---

## 12. Testing Strategy

The architecture is built so the risky logic is **pure and fast to test**.

1. **`shared` unit tests (the backbone).** All formulas (mass/cost/hp/thrust/attack/power/speed) and `validate` (connectivity, required blocks, power balance) are pure functions — table‑driven tests with fixtures. Runs in milliseconds in CI.
2. **Pure `sim` tests.** `movement`, `combat::resolve`, and `economy` operate on plain structs. Test a scripted battle: fixed ships in → exact hp + `CombatEvent` list out. This is where determinism is proven.
3. **Golden tick tests.** Construct an initial world (plain structs) + a fixed order list, run the pure tick core, and assert the resulting snapshot equals a checked‑in golden. Re‑running must be bit‑identical (no RNG drift, no order drift). Regenerate goldens intentionally when balance changes.
4. **Reducer/integration tests.** Drive the published module via CLI/SDK in a scratch database: `init` → `create_faction` → `commit_design` → `order_build_ship` → `advance_tick` ×N, asserting via `spacetime sql`. Heavier, run pre‑merge.
5. **Client smoke tests (manual, per milestone).** The acceptance checklists in §10.

> **Why this works:** because `run_tick` delegates to pure functions, ~90% of game correctness is covered by fast deterministic unit tests that never touch the database or the renderer.

---

## 13. Open Questions & Future Work (explicitly out of MVP)

- **Shared persistent universe:** flip on `tick_timer` (scheduled ticks), faction‑scoped RLS, many human identities. The data model and reducers already support it (§1.3, §8.3).
- **Timed ticks & pacing:** real‑time‑between‑turns cadence, pause/resume, variable tick length.
- **Procedural galaxy generation** (MVP hand‑seeds a fixed galaxy).
- **Per‑component damage:** damage maps to specific blocks (the grid is already stored, so this is additive); destroyed blocks change stats/connectivity.
- **Full 24‑orientation block rotation** (MVP is 4‑way yaw).
- **Richer AI:** beyond placeholder move/attack — economy management, design generation, threat response.
- **Diplomacy, logistics chains, fuel networks, research trees, colonization depth.**
- **Client prediction/reconciliation** for hosted latency.
- **Save/load:** SpacetimeDB persists state inherently; "save slots" would be a snapshot/restore feature.

---

## 14. Appendices

### 14.1 Table catalogue

| Table | PK | Key indexes | Public | Notes |
|---|---|---|---|---|
| `game_state` | `id`(=1) | — | yes | singleton: tick + rng_seed |
| `player_account` | `identity` | `faction_id`(uniq) | yes→**private** | identity → faction |
| `faction` | `id` | `name`(uniq) | yes | resources, home system |
| `star_system` | `id` | — | yes | galaxy position |
| `planet` | `id` | `system_id` | yes | economic output |
| `ship_design` | `id` | `faction_id` | yes | immutable; precomputed stats |
| `ship_design_block` | `id` | `design_id` | yes | committed grid |
| `ship` | `id` | `faction_id`, `system_id` | yes | runtime instance |
| `fleet` | `id` | `faction_id`, `system_id` | yes | + transit fields |
| `fleet_ship` | `ship_id` | `fleet_id` | yes | join (1 ship → ≤1 fleet) |
| `order` | `id` | `faction_id` | yes | tick‑resolved commands |
| `ship_design_draft` | `id` | `faction_id` | yes | editor buffer |
| `ship_design_draft_block` | `id` | `draft_id` | yes | live edits |
| `combat_event` | `id` | `tick` | yes | combat log |
| `tick_timer` | `scheduled_id` | — | (scheduled) | empty in MVP |

### 14.2 Enums

- `BlockType` = { Hull, Engine, Weapon, Reactor, Sensor, **CommandCore** }
- `OrderType` = { MoveFleet, Attack, BuildShip, Colonize }
- `OrderStatus` = { Pending, Active, Done, Failed }

### 14.3 Tunable constants (first pass)

| Constant | Value | Meaning |
|---|---:|---|
| `SPEED_SCALE` | 1.0 | galaxy‑distance units covered per unit `speed` per tick |
| `SPEED_MAX` | 20.0 | clamp on `thrust/mass` |
| `BUILD_RATE` | 50 | cost units built per tick (→ `build_ticks`) |
| `FUEL_PER_JUMP` | 10 | fuel consumed per system jump |
| Block stats | §6.2 | mass/cost/hp/thrust/attack/power per block |

### 14.4 Glossary

- **Tick** — one discrete simulation step; advances only via `advance_tick` (MVP) or `scheduled_tick` (shared).
- **Reducer** — a SpacetimeDB transaction function; the only thing that writes state.
- **Draft vs Design** — a draft is the mutable editor buffer; a design is the immutable committed blueprint.
- **Ship vs Design** — a design is a blueprint; a ship is a runtime instance built from one.
- **Subscription** — a client query whose results stream to the client and fire row callbacks.
- **RLS** — row‑level security; per‑faction visibility filters for the shared phase.

---

*End of TDD v0.1. Update `schema_version` and this document together when tables, reducers, or the tick pipeline change.*
