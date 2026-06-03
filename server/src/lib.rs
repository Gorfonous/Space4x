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

use spacetimedb::{Identity, ReducerContext, ScheduleAt, Table, Timestamp};

// The wire enums BlockType / OrderType / OrderStatus are defined once in the
// shared crate (single source of truth). The `spacetimedb-types` feature there
// gives them their `SpacetimeType` derives so they can be used in tables here.
use starframe_shared::{build_ticks, ship_stats, BlockPlacement, BlockType, OrderStatus, OrderType};

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
// Tables — simulation run log (the client's "batch done" signal)
// ──────────────────────────────────────────────────────────────────────────

/// Appended once per advance_ticks / advance_days call, after the batch
/// commits. The client subscribes to this table; a new row (together with the
/// reducer-completion callback) is the explicit "your requested ticks are
/// processed" signal. The batch runs in one atomic transaction, so the client
/// observes the final world state and this row together.
#[spacetimedb::table(accessor = sim_run, public)]
pub struct SimRun {
    #[primary_key]
    #[auto_inc]
    pub run_id: u64,
    pub requested_ticks: u64,
    pub from_tick: u64,
    pub to_tick: u64,
    pub completed_at: Timestamp,
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

    // The client is read-only and cannot create a faction, so the starting
    // world — the player's faction AND an AI faction, each with a home system —
    // is seeded here. Remaining systems stay neutral and claimable later.
    let player_home = system_ids[0];
    let ai_home = system_ids[GALAXY_SYSTEMS as usize / 2];

    let player = ctx.db.faction().insert(Faction {
        id: 0,
        name: "Terran Union".to_string(),
        is_ai: false,
        minerals: 500,
        energy: 500,
        research: 0,
        home_system_id: player_home,
    });
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
        let owner = if sid == player_home {
            Some(player.id)
        } else if sid == ai_home {
            Some(ai.id)
        } else {
            None
        };
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

    // Starter assets per faction: a default warship design + a home fleet with
    // one ship, so build / move / combat are exercisable immediately.
    seed_starter_assets(ctx, player.id, player_home);
    seed_starter_assets(ctx, ai.id, ai_home);

    log::info!(
        "init: seeded {} systems; player faction {} (home {}), AI faction {} (home {})",
        GALAXY_SYSTEMS,
        player.id,
        player_home,
        ai.id,
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

/// NOT part of the client contract yet (the client is read-only; the player's
/// faction is seeded in `init`). Kept for tests/admin and as a future client
/// message: claim a faction for the caller, taking the lowest-id unowned system
/// as a home (and its planet) with starting resources.
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
// Tick — the ONLY client→server command for now (everything else is read-only)
// ──────────────────────────────────────────────────────────────────────────

/// Bounds the work done in one transaction.
const MAX_TICKS_PER_CALL: u64 = 100_000;

/// The sole client command: process `num_ticks` ticks. Records a `sim_run` row
/// as the completion signal. The whole batch is one atomic transaction, so the
/// client observes the final state and the new sim_run row together.
#[spacetimedb::reducer]
pub fn advance_ticks(ctx: &ReducerContext, num_ticks: u64) -> Result<(), String> {
    do_advance(ctx, num_ticks)
}

/// Convenience matching the design example ("go forward one day" = one day ×
/// ticks-per-day). Expands days into ticks via the shared TICKS_PER_DAY.
#[spacetimedb::reducer]
pub fn advance_days(ctx: &ReducerContext, days: u64) -> Result<(), String> {
    let ticks = days
        .checked_mul(starframe_shared::TICKS_PER_DAY)
        .ok_or("days value too large")?;
    do_advance(ctx, ticks)
}

/// Shared phase: enable by inserting a `tick_timer` row. One tick per fire.
#[spacetimedb::reducer]
pub fn scheduled_tick(ctx: &ReducerContext, _timer: TickTimer) -> Result<(), String> {
    run_tick(ctx)
}

/// Run `num_ticks` deterministic ticks and append the completion record.
fn do_advance(ctx: &ReducerContext, num_ticks: u64) -> Result<(), String> {
    if num_ticks == 0 {
        return Err("num_ticks must be at least 1".to_string());
    }
    if num_ticks > MAX_TICKS_PER_CALL {
        return Err(format!(
            "num_ticks {num_ticks} exceeds the per-call cap of {MAX_TICKS_PER_CALL}"
        ));
    }
    let from_tick = current_tick(ctx)?;
    for _ in 0..num_ticks {
        run_tick(ctx)?;
    }
    let to_tick = current_tick(ctx)?;
    ctx.db.sim_run().insert(SimRun {
        run_id: 0,
        requested_ticks: num_ticks,
        from_tick,
        to_tick,
        completed_at: ctx.timestamp,
    });
    log::info!("advance: processed {num_ticks} tick(s) ({from_tick} -> {to_tick})");
    Ok(())
}

fn current_tick(ctx: &ReducerContext) -> Result<u64, String> {
    ctx.db
        .game_state()
        .id()
        .find(1)
        .map(|gs| gs.current_tick)
        .ok_or_else(|| "game state not initialized".to_string())
}

/// One deterministic simulation step. Phase order matters: arrivals before
/// combat (a fleet arriving fights this tick), combat before economy, builds
/// last (a freshly built ship doesn't act until next tick).
fn run_tick(ctx: &ReducerContext) -> Result<(), String> {
    let now = current_tick(ctx)?;
    movement_phase(ctx, now);
    combat_phase(ctx, now);
    economy_phase(ctx);
    build_phase(ctx, now);
    cleanup_phase(ctx);

    // ADVANCE — bump the clock and the RNG seed (seed reserved for future
    // stochastic rules; the MVP economy/combat are fully deterministic).
    let mut gs = ctx
        .db
        .game_state()
        .id()
        .find(1)
        .ok_or("game state not initialized")?;
    gs.current_tick += 1;
    gs.rng_seed = splitmix64(gs.rng_seed);
    ctx.db.game_state().id().update(gs);
    Ok(())
}

/// MOVEMENT — fleets in transit that reach their arrival tick relocate (with
/// every member ship) to the destination system.
fn movement_phase(ctx: &ReducerContext, now: u64) {
    let mut fleets: Vec<Fleet> = ctx.db.fleet().iter().collect();
    fleets.sort_by_key(|f| f.id);
    for f in fleets {
        let (Some(dest), Some(arr)) = (f.dest_system_id, f.arrival_tick) else {
            continue;
        };
        if arr > now + 1 {
            continue;
        }
        let members: Vec<FleetShip> = ctx.db.fleet_ship().fleet_id().filter(f.id).collect();
        for fs in members {
            if let Some(ship) = ctx.db.ship().id().find(fs.ship_id) {
                ctx.db.ship().id().update(Ship {
                    system_id: dest,
                    ..ship
                });
            }
        }
        ctx.db.fleet().id().update(Fleet {
            system_id: dest,
            dest_system_id: None,
            arrival_tick: None,
            ..f
        });
    }
}

/// COMBAT — in any system holding living ships of 2+ factions, every armed ship
/// fires once at the lowest-id enemy. Deterministic: sorted by id, no RNG.
fn combat_phase(ctx: &ReducerContext, now: u64) {
    use std::collections::HashMap;

    let attack_of: HashMap<u64, i64> = ctx
        .db
        .ship_design()
        .iter()
        .map(|d| (d.id, d.attack))
        .collect();

    let mut ships: Vec<Ship> = ctx.db.ship().iter().collect();
    ships.sort_by_key(|s| s.id);
    let mut hp: Vec<i64> = ships.iter().map(|s| s.hp).collect();

    let mut by_system: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, s) in ships.iter().enumerate() {
        by_system.entry(s.system_id).or_default().push(i);
    }
    let mut systems: Vec<u64> = by_system.keys().copied().collect();
    systems.sort();

    for sys in systems {
        let idxs = by_system.get(&sys).cloned().unwrap_or_default();
        let mut factions: Vec<u64> = idxs.iter().map(|&i| ships[i].faction_id).collect();
        factions.sort();
        factions.dedup();
        if factions.len() < 2 {
            continue;
        }
        for &attacker in &idxs {
            if hp[attacker] <= 0 {
                continue;
            }
            let atk = *attack_of.get(&ships[attacker].design_id).unwrap_or(&0);
            if atk <= 0 {
                continue;
            }
            let target = idxs
                .iter()
                .copied()
                .find(|&t| ships[t].faction_id != ships[attacker].faction_id && hp[t] > 0);
            if let Some(t) = target {
                hp[t] -= atk;
                ctx.db.combat_event().insert(CombatEvent {
                    id: 0,
                    tick: now + 1,
                    system_id: sys,
                    attacker_ship_id: ships[attacker].id,
                    defender_ship_id: ships[t].id,
                    attacker_faction_id: ships[attacker].faction_id,
                    defender_faction_id: ships[t].faction_id,
                    damage_dealt: atk,
                    destroyed: hp[t] <= 0,
                });
            }
        }
    }

    // Persist hp changes once.
    for (i, s) in ships.into_iter().enumerate() {
        if hp[i] != s.hp {
            ctx.db.ship().id().update(Ship { hp: hp[i], ..s });
        }
    }
}

/// ECONOMY — each owned planet contributes to its owner faction. Stable
/// iteration by id keeps the tick deterministic.
fn economy_phase(ctx: &ReducerContext) {
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
}

/// BUILD — complete BuildShip orders whose timer elapsed: spawn the ship into
/// its target fleet at that fleet's system.
fn build_phase(ctx: &ReducerContext, now: u64) {
    let mut orders: Vec<Order> = ctx
        .db
        .orders()
        .iter()
        .filter(|o| o.order_type == OrderType::BuildShip && o.status == OrderStatus::Active)
        .collect();
    orders.sort_by_key(|o| o.id);
    for o in orders {
        if !o.complete_tick.map_or(false, |c| c <= now + 1) {
            continue;
        }
        if let (Some(design_id), Some(fleet_id)) = (o.target_id, o.fleet_id) {
            if let (Some(design), Some(fleet)) = (
                ctx.db.ship_design().id().find(design_id),
                ctx.db.fleet().id().find(fleet_id),
            ) {
                let ship = ctx.db.ship().insert(Ship {
                    id: 0,
                    design_id: design.id,
                    faction_id: design.faction_id,
                    system_id: fleet.system_id,
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    hp: design.max_hp,
                    fuel: 100,
                });
                ctx.db.fleet_ship().insert(FleetShip {
                    ship_id: ship.id,
                    fleet_id: fleet.id,
                });
            }
        }
        ctx.db.orders().id().update(Order {
            status: OrderStatus::Done,
            ..o
        });
    }
}

/// CLEANUP — delete destroyed ships and their fleet membership.
fn cleanup_phase(ctx: &ReducerContext) {
    let dead: Vec<u64> = ctx
        .db
        .ship()
        .iter()
        .filter(|s| s.hp <= 0)
        .map(|s| s.id)
        .collect();
    for ship_id in dead {
        ctx.db.fleet_ship().ship_id().delete(ship_id);
        ctx.db.ship().id().delete(ship_id);
    }
}

/// Deterministic PRNG step (SplitMix64). Used to advance `GameState.rng_seed`.
fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ──────────────────────────────────────────────────────────────────────────
// Orders (gameplay commands). Reference entities by id; the entity's faction
// determines whose resources/ships are affected. Proper caller→faction auth is
// a multiplayer concern, tracked for the shared phase.
// ──────────────────────────────────────────────────────────────────────────

/// Queue a ship build into `fleet_id`: deduct the design's mineral cost now and
/// complete after `build_ticks(cost)`. The ship joins the fleet when done.
#[spacetimedb::reducer]
pub fn order_build_ship(ctx: &ReducerContext, design_id: u64, fleet_id: u64) -> Result<(), String> {
    let design = ctx
        .db
        .ship_design()
        .id()
        .find(design_id)
        .ok_or("design not found")?;
    let fleet = ctx.db.fleet().id().find(fleet_id).ok_or("fleet not found")?;
    if fleet.faction_id != design.faction_id {
        return Err("fleet and design belong to different factions".to_string());
    }
    let mut faction = ctx
        .db
        .faction()
        .id()
        .find(design.faction_id)
        .ok_or("faction not found")?;
    if faction.minerals < design.total_cost {
        return Err(format!(
            "insufficient minerals: need {}, have {}",
            design.total_cost, faction.minerals
        ));
    }
    faction.minerals -= design.total_cost;
    ctx.db.faction().id().update(faction);

    let now = current_tick(ctx)?;
    ctx.db.orders().insert(Order {
        id: 0,
        faction_id: design.faction_id,
        order_type: OrderType::BuildShip,
        status: OrderStatus::Active,
        target_id: Some(design_id),
        target_system_id: None,
        ship_id: None,
        fleet_id: Some(fleet_id),
        created_tick: now,
        complete_tick: Some(now + build_ticks(design.total_cost)),
    });
    Ok(())
}

/// Order a fleet to move to another system. Arrival is ceil(distance / speed)
/// ticks away, where speed is the slowest member ship's thrust/mass.
#[spacetimedb::reducer]
pub fn order_move_fleet(
    ctx: &ReducerContext,
    fleet_id: u64,
    dest_system_id: u64,
) -> Result<(), String> {
    let fleet = ctx.db.fleet().id().find(fleet_id).ok_or("fleet not found")?;
    if dest_system_id == fleet.system_id {
        return Err("fleet is already in that system".to_string());
    }
    let src = ctx
        .db
        .star_system()
        .id()
        .find(fleet.system_id)
        .ok_or("source system missing")?;
    let dest = ctx
        .db
        .star_system()
        .id()
        .find(dest_system_id)
        .ok_or("destination system not found")?;
    let speed = fleet_speed(ctx, fleet_id);
    if speed <= 0.0 {
        return Err("fleet has no propulsion (no ships, or no engines)".to_string());
    }
    let dx = dest.x - src.x;
    let dy = dest.y - src.y;
    let dz = dest.z - src.z;
    let dist = (dx * dx + dy * dy + dz * dz).sqrt();
    let ticks = ((dist / (speed * starframe_shared::SPEED_SCALE)).ceil() as u64).max(1);
    let now = current_tick(ctx)?;
    ctx.db.fleet().id().update(Fleet {
        dest_system_id: Some(dest_system_id),
        arrival_tick: Some(now + ticks),
        ..fleet
    });
    Ok(())
}

/// Slowest member ship's speed (thrust/mass); 0 if the fleet is empty.
fn fleet_speed(ctx: &ReducerContext, fleet_id: u64) -> f32 {
    let mut slowest: Option<f32> = None;
    for fs in ctx.db.fleet_ship().fleet_id().filter(fleet_id) {
        if let Some(ship) = ctx.db.ship().id().find(fs.ship_id) {
            if let Some(design) = ctx.db.ship_design().id().find(ship.design_id) {
                let s = if design.total_mass > 0.0 {
                    design.thrust / design.total_mass
                } else {
                    0.0
                };
                slowest = Some(slowest.map_or(s, |m| m.min(s)));
            }
        }
    }
    slowest.unwrap_or(0.0)
}

// ──────────────────────────────────────────────────────────────────────────
// Starter assets (seeded in init)
// ──────────────────────────────────────────────────────────────────────────

fn default_warship_blocks() -> Vec<BlockPlacement> {
    vec![
        BlockPlacement::new(0, 0, 0, BlockType::CommandCore),
        BlockPlacement::new(1, 0, 0, BlockType::Engine),
        BlockPlacement::new(-1, 0, 0, BlockType::Reactor),
        BlockPlacement::new(0, 1, 0, BlockType::Weapon),
        BlockPlacement::new(0, -1, 0, BlockType::Hull),
    ]
}

/// Insert a default "Scout" design for `faction_id`, plus a home fleet with one
/// ship of that design at `system_id`.
fn seed_starter_assets(ctx: &ReducerContext, faction_id: u64, system_id: u64) {
    let blocks = default_warship_blocks();
    let stats = ship_stats(&blocks);
    let design = ctx.db.ship_design().insert(ShipDesign {
        id: 0,
        faction_id,
        name: "Scout".to_string(),
        total_mass: stats.mass,
        total_cost: stats.cost,
        max_hp: stats.max_hp,
        thrust: stats.thrust,
        attack: stats.attack,
        block_count: stats.block_count,
        created_tick: 0,
    });
    for b in &blocks {
        ctx.db.ship_design_block().insert(ShipDesignBlock {
            id: 0,
            design_id: design.id,
            x: b.x,
            y: b.y,
            z: b.z,
            block_type: b.block_type,
            rotation: b.rotation,
        });
    }
    let fleet = ctx.db.fleet().insert(Fleet {
        id: 0,
        faction_id,
        system_id,
        name: "Home Fleet".to_string(),
        dest_system_id: None,
        arrival_tick: None,
    });
    let ship = ctx.db.ship().insert(Ship {
        id: 0,
        design_id: design.id,
        faction_id,
        system_id,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        hp: stats.max_hp,
        fuel: 100,
    });
    ctx.db.fleet_ship().insert(FleetShip {
        ship_id: ship.id,
        fleet_id: fleet.id,
    });
}

// ──────────────────────────────────────────────────────────────────────────
// Ship designer (drafts + atomic commit). A draft is the live editor buffer;
// commit_design validates it (shared) and snapshots it into an immutable design.
// ──────────────────────────────────────────────────────────────────────────

/// Start a new, empty draft for a faction.
#[spacetimedb::reducer]
pub fn create_draft(ctx: &ReducerContext, faction_id: u64, name: String) -> Result<(), String> {
    if ctx.db.faction().id().find(faction_id).is_none() {
        return Err("faction not found".to_string());
    }
    if name.trim().is_empty() {
        return Err("draft name is required".to_string());
    }
    ctx.db.ship_design_draft().insert(ShipDesignDraft {
        id: 0,
        faction_id,
        name,
        updated_at: ctx.timestamp,
    });
    Ok(())
}

/// Place (or overwrite) the block at (x, y, z) in a draft.
#[spacetimedb::reducer]
pub fn place_block(
    ctx: &ReducerContext,
    draft_id: u64,
    x: i32,
    y: i32,
    z: i32,
    block_type: BlockType,
    rotation: u8,
) -> Result<(), String> {
    let draft = ctx
        .db
        .ship_design_draft()
        .id()
        .find(draft_id)
        .ok_or("draft not found")?;
    let existing: Vec<u64> = ctx
        .db
        .ship_design_draft_block()
        .draft_id()
        .filter(draft_id)
        .filter(|b| b.x == x && b.y == y && b.z == z)
        .map(|b| b.id)
        .collect();
    for id in existing {
        ctx.db.ship_design_draft_block().id().delete(id);
    }
    ctx.db.ship_design_draft_block().insert(ShipDesignDraftBlock {
        id: 0,
        draft_id,
        x,
        y,
        z,
        block_type,
        rotation: rotation % 4,
    });
    ctx.db.ship_design_draft().id().update(ShipDesignDraft {
        updated_at: ctx.timestamp,
        ..draft
    });
    Ok(())
}

/// Remove the block at (x, y, z) from a draft (no-op if the cell is empty).
#[spacetimedb::reducer]
pub fn remove_block(
    ctx: &ReducerContext,
    draft_id: u64,
    x: i32,
    y: i32,
    z: i32,
) -> Result<(), String> {
    let draft = ctx
        .db
        .ship_design_draft()
        .id()
        .find(draft_id)
        .ok_or("draft not found")?;
    let existing: Vec<u64> = ctx
        .db
        .ship_design_draft_block()
        .draft_id()
        .filter(draft_id)
        .filter(|b| b.x == x && b.y == y && b.z == z)
        .map(|b| b.id)
        .collect();
    for id in existing {
        ctx.db.ship_design_draft_block().id().delete(id);
    }
    ctx.db.ship_design_draft().id().update(ShipDesignDraft {
        updated_at: ctx.timestamp,
        ..draft
    });
    Ok(())
}

/// Validate a draft (connectivity + required blocks + power, via the shared
/// crate) and, if valid, snapshot it into an immutable ShipDesign + blocks.
/// The draft is left intact so the player can keep iterating.
#[spacetimedb::reducer]
pub fn commit_design(ctx: &ReducerContext, draft_id: u64, name: String) -> Result<(), String> {
    let draft = ctx
        .db
        .ship_design_draft()
        .id()
        .find(draft_id)
        .ok_or("draft not found")?;
    if name.trim().is_empty() {
        return Err("design name is required".to_string());
    }

    let blocks: Vec<BlockPlacement> = ctx
        .db
        .ship_design_draft_block()
        .draft_id()
        .filter(draft_id)
        .map(|b| BlockPlacement {
            x: b.x,
            y: b.y,
            z: b.z,
            block_type: b.block_type,
            rotation: b.rotation,
        })
        .collect();

    let report = starframe_shared::validate(&blocks);
    if !report.is_valid {
        return Err(report.problems.join("; "));
    }
    let stats = ship_stats(&blocks);

    let design = ctx.db.ship_design().insert(ShipDesign {
        id: 0,
        faction_id: draft.faction_id,
        name,
        total_mass: stats.mass,
        total_cost: stats.cost,
        max_hp: stats.max_hp,
        thrust: stats.thrust,
        attack: stats.attack,
        block_count: stats.block_count,
        created_tick: current_tick(ctx)?,
    });
    for b in &blocks {
        ctx.db.ship_design_block().insert(ShipDesignBlock {
            id: 0,
            design_id: design.id,
            x: b.x,
            y: b.y,
            z: b.z,
            block_type: b.block_type,
            rotation: b.rotation,
        });
    }
    log::info!(
        "commit_design: faction {} -> design {} ({} blocks, cost {}, hp {}, attack {})",
        draft.faction_id,
        design.id,
        stats.block_count,
        stats.cost,
        stats.max_hp,
        stats.attack
    );
    Ok(())
}
