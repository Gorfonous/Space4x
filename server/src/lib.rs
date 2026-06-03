//! Starframe (Space4x) — SpacetimeDB server module.
//!
//! This is the initial server-side slice (Milestone 1, Core Engine). It defines
//! the core data model from the TDD, seeds a small galaxy on `init`, lets a
//! player claim a faction, and advances the simulation one tick at a time.
//!
//! Design rules carried from the TDD:
//! - The server is the ONLY writer; clients call reducers and read via subscriptions.
//! - The tick is a deterministic function of state (stable iteration by id, seeded
//!   RNG persisted in `GameState`) so it can later be golden-tested and run from a
//!   scheduled reducer unchanged.
//! - State is identity-addressed and faction-scoped from day one (single-player now,
//!   shared persistent universe later).

use spacetimedb::{Identity, ReducerContext, ScheduleAt, SpacetimeType, Table, Timestamp};

// ──────────────────────────────────────────────────────────────────────────
// Enums
// ──────────────────────────────────────────────────────────────────────────

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
}

#[derive(SpacetimeType, Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderStatus {
    Pending,
    Active,
    Done,
    Failed,
}

// ──────────────────────────────────────────────────────────────────────────
// Tables — singletons & accounts
// ──────────────────────────────────────────────────────────────────────────

/// Single-row table (id is always 1): the simulation clock + RNG seed.
#[spacetimedb::table(accessor = game_state, public)]
pub struct GameState {
    #[primary_key]
    pub id: u64,
    pub current_tick: u64,
    pub rng_seed: u64,
    pub schema_version: u32,
}

/// Maps a connection Identity to the faction it controls.
#[spacetimedb::table(accessor = player_account, public)]
pub struct PlayerAccount {
    #[primary_key]
    pub identity: Identity,
    #[unique]
    pub faction_id: u64,
    pub created_at: Timestamp,
}

// ──────────────────────────────────────────────────────────────────────────
// Tables — empire & galaxy
// ──────────────────────────────────────────────────────────────────────────

#[spacetimedb::table(accessor = faction, public)]
pub struct Faction {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[unique]
    pub name: String,
    pub is_ai: bool,
    pub minerals: i64,
    pub energy: i64,
    pub research: i64,
    pub home_system_id: u64,
}

#[spacetimedb::table(accessor = star_system, public)]
pub struct StarSystem {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub name: String,
    pub owner_faction_id: Option<u64>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[spacetimedb::table(accessor = planet, public)]
pub struct Planet {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub owner_faction_id: Option<u64>,
    pub population: i64,
    pub minerals_output: i64,
    pub energy_output: i64,
    pub research_output: i64,
}

// ──────────────────────────────────────────────────────────────────────────
// Tables — ship designs (the editor's committed output)
// ──────────────────────────────────────────────────────────────────────────

#[spacetimedb::table(accessor = ship_design, public)]
pub struct ShipDesign {
    #[primary_key]
    #[auto_inc]
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

#[spacetimedb::table(accessor = ship_design_block, public)]
pub struct ShipDesignBlock {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub design_id: u64,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block_type: BlockType,
    pub rotation: u8,
}

// ──────────────────────────────────────────────────────────────────────────
// Tables — runtime ships & fleets
// ──────────────────────────────────────────────────────────────────────────

#[spacetimedb::table(accessor = ship, public)]
pub struct Ship {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub design_id: u64,
    #[index(btree)]
    pub faction_id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub hp: i64,
    pub fuel: i64,
}

#[spacetimedb::table(accessor = fleet, public)]
pub struct Fleet {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    #[index(btree)]
    pub system_id: u64,
    pub name: String,
    pub dest_system_id: Option<u64>,
    pub arrival_tick: Option<u64>,
}

/// A ship is in at most one fleet → `ship_id` is the primary key.
#[spacetimedb::table(accessor = fleet_ship, public)]
pub struct FleetShip {
    #[primary_key]
    pub ship_id: u64,
    #[index(btree)]
    pub fleet_id: u64,
}

// ──────────────────────────────────────────────────────────────────────────
// Tables — orders, drafts, combat log
// ──────────────────────────────────────────────────────────────────────────

/// `orders` (not `order`) avoids the SQL reserved word for `spacetime sql`.
#[spacetimedb::table(accessor = orders, public)]
pub struct Order {
    #[primary_key]
    #[auto_inc]
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
    pub complete_tick: Option<u64>,
}

#[spacetimedb::table(accessor = ship_design_draft, public)]
pub struct ShipDesignDraft {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub faction_id: u64,
    pub name: String,
    pub updated_at: Timestamp,
}

#[spacetimedb::table(accessor = ship_design_draft_block, public)]
pub struct ShipDesignDraftBlock {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[index(btree)]
    pub draft_id: u64,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block_type: BlockType,
    pub rotation: u8,
}

#[spacetimedb::table(accessor = combat_event, public)]
pub struct CombatEvent {
    #[primary_key]
    #[auto_inc]
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

// ──────────────────────────────────────────────────────────────────────────
// Tables — scheduling seam (DORMANT in MVP)
// ──────────────────────────────────────────────────────────────────────────

/// Single→shared seam: when we move to a timed shared universe, insert one row
/// here and the scheduler invokes `scheduled_tick`. In single-player this table
/// stays empty and the player drives ticks via `advance_tick`.
#[spacetimedb::table(accessor = tick_timer, scheduled(scheduled_tick), public)]
pub struct TickTimer {
    #[primary_key]
    #[auto_inc]
    pub scheduled_id: u64,
    pub scheduled_at: ScheduleAt,
}

// ──────────────────────────────────────────────────────────────────────────
// Lifecycle
// ──────────────────────────────────────────────────────────────────────────

const GALAXY_SYSTEMS: u64 = 8;

#[spacetimedb::reducer(init)]
pub fn init(ctx: &ReducerContext) {
    ctx.db.game_state().insert(GameState {
        id: 1,
        current_tick: 0,
        rng_seed: 0x9E37_79B9_7F4A_7C15,
        schema_version: 1,
    });

    // Deterministic galaxy: systems on a ring, one planet each.
    let mut system_ids: Vec<u64> = Vec::new();
    for i in 0..GALAXY_SYSTEMS {
        let angle = (i as f32) / (GALAXY_SYSTEMS as f32) * std::f32::consts::TAU;
        let radius = 100.0_f32;
        let s = ctx.db.star_system().insert(StarSystem {
            id: 0,
            name: format!("System {}", i + 1),
            owner_faction_id: None,
            x: radius * angle.cos(),
            y: radius * angle.sin(),
            z: 0.0,
        });
        system_ids.push(s.id);
    }

    // One AI faction with a home system; the human faction is created on demand
    // via `create_faction`. Remaining systems are neutral and claimable.
    let ai_home = system_ids[GALAXY_SYSTEMS as usize / 2];
    let ai = ctx.db.faction().insert(Faction {
        id: 0,
        name: "AI Raiders".to_string(),
        is_ai: true,
        minerals: 1_000,
        energy: 1_000,
        research: 0,
        home_system_id: ai_home,
    });

    for &sid in &system_ids {
        let owner = if sid == ai_home { Some(ai.id) } else { None };
        if owner.is_some() {
            if let Some(sys) = ctx.db.star_system().id().find(sid) {
                ctx.db.star_system().id().update(StarSystem {
                    owner_faction_id: owner,
                    ..sys
                });
            }
        }
        ctx.db.planet().insert(Planet {
            id: 0,
            system_id: sid,
            owner_faction_id: owner,
            population: if owner.is_some() { 100 } else { 0 },
            minerals_output: 50,
            energy_output: 40,
            research_output: 10,
        });
    }

    log::info!(
        "init: seeded {} systems, 1 AI faction (home {})",
        GALAXY_SYSTEMS,
        ai_home
    );
}

#[spacetimedb::reducer(client_connected)]
pub fn on_connect(ctx: &ReducerContext) {
    log::info!("client connected: {:?}", ctx.sender());
}

#[spacetimedb::reducer(client_disconnected)]
pub fn on_disconnect(ctx: &ReducerContext) {
    log::info!("client disconnected: {:?}", ctx.sender());
}

// ──────────────────────────────────────────────────────────────────────────
// Faction & account
// ──────────────────────────────────────────────────────────────────────────

/// Claim a human faction for the calling identity, taking the lowest-id
/// unowned system as a home (and its planet) with starting resources.
#[spacetimedb::reducer]
pub fn create_faction(ctx: &ReducerContext, name: String) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("faction name is required".to_string());
    }
    if ctx.db.player_account().identity().find(ctx.sender()).is_some() {
        return Err("this identity already controls a faction".to_string());
    }

    let mut unowned: Vec<StarSystem> = ctx
        .db
        .star_system()
        .iter()
        .filter(|s| s.owner_faction_id.is_none())
        .collect();
    unowned.sort_by_key(|s| s.id);
    let home = unowned
        .into_iter()
        .next()
        .ok_or("no unowned system available to settle")?;
    let home_id = home.id;

    let faction = ctx.db.faction().try_insert(Faction {
        id: 0,
        name,
        is_ai: false,
        minerals: 500,
        energy: 500,
        research: 0,
        home_system_id: home_id,
    })?;

    ctx.db.star_system().id().update(StarSystem {
        owner_faction_id: Some(faction.id),
        ..home
    });

    for p in ctx.db.planet().system_id().filter(home_id) {
        ctx.db.planet().id().update(Planet {
            owner_faction_id: Some(faction.id),
            population: 100,
            ..p
        });
    }

    ctx.db.player_account().insert(PlayerAccount {
        identity: ctx.sender(),
        faction_id: faction.id,
        created_at: ctx.timestamp,
    });

    log::info!(
        "create_faction: faction {} settled system {} for {:?}",
        faction.id,
        home_id,
        ctx.sender()
    );
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// Tick
// ──────────────────────────────────────────────────────────────────────────

/// MVP: the player's "Advance Turn" button calls this.
#[spacetimedb::reducer]
pub fn advance_tick(ctx: &ReducerContext) -> Result<(), String> {
    run_tick(ctx)
}

/// Shared phase: enable by inserting a `tick_timer` row. Identical logic.
#[spacetimedb::reducer]
pub fn scheduled_tick(ctx: &ReducerContext, _timer: TickTimer) -> Result<(), String> {
    run_tick(ctx)
}

/// One simulation step. Phases are ordered and deterministic.
/// MVP implements ECONOMY + ADVANCE; movement/combat/build land next.
fn run_tick(ctx: &ReducerContext) -> Result<(), String> {
    let mut gs = ctx
        .db
        .game_state()
        .id()
        .find(1)
        .ok_or("game state not initialized")?;

    // ECONOMY — each owned planet contributes to its owner faction.
    // Stable iteration by id keeps the tick deterministic.
    let mut planets: Vec<Planet> = ctx.db.planet().iter().collect();
    planets.sort_by_key(|p| p.id);
    for p in planets {
        if let Some(owner) = p.owner_faction_id {
            if let Some(f) = ctx.db.faction().id().find(owner) {
                ctx.db.faction().id().update(Faction {
                    minerals: f.minerals + p.minerals_output,
                    energy: f.energy + p.energy_output,
                    research: f.research + p.research_output,
                    ..f
                });
            }
        }
    }

    // ADVANCE — bump the clock and the RNG seed (seed reserved for future
    // stochastic rules; MVP economy/combat are fully deterministic).
    gs.current_tick += 1;
    gs.rng_seed = splitmix64(gs.rng_seed);
    let tick = gs.current_tick;
    ctx.db.game_state().id().update(gs);
    log::info!("advance_tick: now at tick {}", tick);
    Ok(())
}

/// Deterministic PRNG step (SplitMix64). Used to advance `GameState.rng_seed`.
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
