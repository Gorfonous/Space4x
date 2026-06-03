//! Starframe (Space4x) — shared types and pure game math.
//!
//! Compiled into BOTH the wasm server module and the native client so the ship
//! editor's live preview and the server's commit/resolution compute identical
//! numbers. With the default feature set this crate is pure (no dependencies),
//! so `cargo test -p starframe-shared` runs on the host with no SpacetimeDB or
//! wasm toolchain. Enabling `spacetimedb-types` adds `SpacetimeType` derives so
//! the server can use these enums directly in its table definitions.

// ──────────────────────────────────────────────────────────────────────────
// Wire enums — single source of truth.
// Keep variant ORDER stable: it is part of the SpacetimeDB module schema.
// ──────────────────────────────────────────────────────────────────────────

#[cfg_attr(feature = "spacetimedb-types", derive(spacetimedb::SpacetimeType))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockType {
    Hull,
    Engine,
    Weapon,
    Reactor,
    Sensor,
    CommandCore,
}

#[cfg_attr(feature = "spacetimedb-types", derive(spacetimedb::SpacetimeType))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderType {
    MoveFleet,
    Attack,
    BuildShip,
    Colonize,
}

#[cfg_attr(feature = "spacetimedb-types", derive(spacetimedb::SpacetimeType))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderStatus {
    Pending,
    Active,
    Done,
    Failed,
}

// ──────────────────────────────────────────────────────────────────────────
// Tunable constants (first-pass balance; TDD §14.3).
// ──────────────────────────────────────────────────────────────────────────

/// Simulation ticks in one in-game day. The client advances time in days
/// (advance_days), which the server expands into this many ticks.
pub const TICKS_PER_DAY: u64 = 24;

/// Galaxy-distance units covered per unit of `speed` per tick.
pub const SPEED_SCALE: f32 = 1.0;
/// Clamp on `thrust / mass`.
pub const SPEED_MAX: f32 = 20.0;
/// Cost units built per tick (→ `build_ticks`).
pub const BUILD_RATE: i64 = 50;
/// Fuel consumed per system jump.
pub const FUEL_PER_JUMP: i64 = 10;

// ──────────────────────────────────────────────────────────────────────────
// Per-block constants (TDD §6.2).
// ──────────────────────────────────────────────────────────────────────────

/// Static stats contributed by one block of a given type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockStats {
    pub mass: f32,
    pub cost: i64,
    pub hp: i64,
    pub thrust: f32,
    pub attack: i64,
    /// Net power: reactors produce (positive), consumers draw (negative).
    pub power: i64,
}

pub const fn block_stats(b: BlockType) -> BlockStats {
    match b {
        BlockType::Hull => BlockStats { mass: 1.0, cost: 10, hp: 50, thrust: 0.0, attack: 0, power: 0 },
        BlockType::Engine => BlockStats { mass: 2.0, cost: 25, hp: 20, thrust: 10.0, attack: 0, power: -5 },
        BlockType::Weapon => BlockStats { mass: 1.5, cost: 40, hp: 20, thrust: 0.0, attack: 10, power: -8 },
        BlockType::Reactor => BlockStats { mass: 3.0, cost: 50, hp: 30, thrust: 0.0, attack: 0, power: 20 },
        BlockType::Sensor => BlockStats { mass: 0.5, cost: 20, hp: 10, thrust: 0.0, attack: 0, power: -2 },
        BlockType::CommandCore => BlockStats { mass: 2.0, cost: 100, hp: 100, thrust: 0.0, attack: 0, power: -3 },
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Design model + derived ship stats.
// ──────────────────────────────────────────────────────────────────────────

/// One placed block in a design grid — the input to all design math.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockPlacement {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub block_type: BlockType,
    pub rotation: u8,
}

impl BlockPlacement {
    /// Convenience for tests/tools: a placement with rotation 0.
    pub fn new(x: i32, y: i32, z: i32, block_type: BlockType) -> Self {
        Self { x, y, z, block_type, rotation: 0 }
    }
}

/// Aggregate stats of a whole design.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShipStats {
    pub mass: f32,
    pub cost: i64,
    pub max_hp: i64,
    pub thrust: f32,
    pub attack: i64,
    pub power: i64,
    pub speed: f32,
    pub block_count: i32,
}

/// Sum the per-block contributions and derive speed = clamp(thrust/mass).
pub fn ship_stats(blocks: &[BlockPlacement]) -> ShipStats {
    let mut s = ShipStats {
        mass: 0.0,
        cost: 0,
        max_hp: 0,
        thrust: 0.0,
        attack: 0,
        power: 0,
        speed: 0.0,
        block_count: blocks.len() as i32,
    };
    for b in blocks {
        let bs = block_stats(b.block_type);
        s.mass += bs.mass;
        s.cost += bs.cost;
        s.max_hp += bs.hp;
        s.thrust += bs.thrust;
        s.attack += bs.attack;
        s.power += bs.power;
    }
    s.speed = if s.mass > 0.0 {
        (s.thrust / s.mass * SPEED_SCALE).min(SPEED_MAX)
    } else {
        0.0
    };
    s
}

/// Number of ticks to build a design (used by the BuildShip order, M4).
pub fn build_ticks(cost: i64) -> u64 {
    let ticks = (cost + BUILD_RATE - 1) / BUILD_RATE; // ceil(cost / BUILD_RATE)
    ticks.max(1) as u64
}

// ──────────────────────────────────────────────────────────────────────────
// Validation — connectivity, required blocks, power balance.
// ──────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidationReport {
    pub is_valid: bool,
    pub problems: Vec<String>,
}

/// A design is valid iff: non-empty, no overlapping cells, ≥1 command core,
/// ≥1 engine, power balance ≥ 0, and every block is connected (6-neighbour)
/// to the command core.
pub fn validate(blocks: &[BlockPlacement]) -> ValidationReport {
    use std::collections::HashSet;

    let mut problems: Vec<String> = Vec::new();

    if blocks.is_empty() {
        problems.push("design is empty".to_string());
        return ValidationReport { is_valid: false, problems };
    }

    // No two blocks may share a cell.
    let cells: HashSet<(i32, i32, i32)> = blocks.iter().map(|b| (b.x, b.y, b.z)).collect();
    if cells.len() != blocks.len() {
        problems.push("two or more blocks occupy the same cell".to_string());
    }

    // Required blocks.
    let command_cores = blocks.iter().filter(|b| b.block_type == BlockType::CommandCore).count();
    let engines = blocks.iter().filter(|b| b.block_type == BlockType::Engine).count();
    if command_cores < 1 {
        problems.push("needs at least 1 command core".to_string());
    }
    if engines < 1 {
        problems.push("needs at least 1 engine".to_string());
    }

    // Power balance (reactors must cover consumers).
    let power: i64 = blocks.iter().map(|b| block_stats(b.block_type).power).sum();
    if power < 0 {
        problems.push(format!("insufficient power: net {power} (add a reactor)"));
    }

    // Connectivity from a command core.
    if command_cores >= 1 {
        let detached = count_disconnected(blocks, &cells);
        if detached > 0 {
            problems.push(format!("{detached} block(s) are not connected to the command core"));
        }
    }

    ValidationReport { is_valid: problems.is_empty(), problems }
}

/// Count cells unreachable from the first command core over 6-neighbour
/// adjacency. Deterministic (returns a count; iteration order is irrelevant).
fn count_disconnected(blocks: &[BlockPlacement], cells: &std::collections::HashSet<(i32, i32, i32)>) -> usize {
    use std::collections::{HashSet, VecDeque};

    let start = match blocks.iter().find(|b| b.block_type == BlockType::CommandCore) {
        Some(b) => (b.x, b.y, b.z),
        None => return 0,
    };

    const NEIGHBOURS: [(i32, i32, i32); 6] =
        [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)];

    let mut seen: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut queue: VecDeque<(i32, i32, i32)> = VecDeque::new();
    seen.insert(start);
    queue.push_back(start);
    while let Some((x, y, z)) = queue.pop_front() {
        for (dx, dy, dz) in NEIGHBOURS {
            let n = (x + dx, y + dy, z + dz);
            if cells.contains(&n) && seen.insert(n) {
                queue.push_back(n);
            }
        }
    }

    cells.len() - seen.len()
}

// ──────────────────────────────────────────────────────────────────────────
// Tests (run on the host with the default, pure feature set).
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i32, y: i32, z: i32, b: BlockType) -> BlockPlacement {
        BlockPlacement::new(x, y, z, b)
    }

    #[test]
    fn block_constants_sane() {
        assert_eq!(block_stats(BlockType::Reactor).power, 20);
        assert_eq!(block_stats(BlockType::CommandCore).hp, 100);
        assert_eq!(block_stats(BlockType::Weapon).attack, 10);
    }

    #[test]
    fn ship_stats_sum_and_speed() {
        // A connected, valid little ship laid out along the x-axis.
        let ship = [
            p(-1, 0, 0, BlockType::Hull),
            p(0, 0, 0, BlockType::CommandCore),
            p(1, 0, 0, BlockType::Engine),
            p(2, 0, 0, BlockType::Reactor),
        ];
        let s = ship_stats(&ship);
        assert_eq!(s.block_count, 4);
        assert_eq!(s.mass, 1.0 + 2.0 + 2.0 + 3.0); // 8.0
        assert_eq!(s.cost, 10 + 100 + 25 + 50); // 185
        assert_eq!(s.max_hp, 50 + 100 + 20 + 30); // 200
        assert_eq!(s.thrust, 10.0);
        assert_eq!(s.attack, 0);
        assert_eq!(s.power, 0 - 3 - 5 + 20); // +12
        assert_eq!(s.speed, 10.0 / 8.0); // 1.25
    }

    #[test]
    fn speed_is_thrust_over_mass() {
        // 4 engines + a command core: thrust 40, mass 2 + 4*2 = 10 → 4.0,
        // comfortably under the defensive SPEED_MAX cap (unreachable with the
        // current block constants, since thrust/mass asymptotes to ~5).
        let mut blocks = vec![p(0, 0, 0, BlockType::CommandCore)];
        for i in 1..=4 {
            blocks.push(p(i, 0, 0, BlockType::Engine));
        }
        let s = ship_stats(&blocks);
        assert_eq!(s.thrust, 40.0);
        assert_eq!(s.mass, 10.0);
        assert_eq!(s.speed, 4.0);
        assert!(s.speed <= SPEED_MAX);
    }

    #[test]
    fn valid_minimal_ship() {
        let ship = [
            p(0, 0, 0, BlockType::CommandCore),
            p(1, 0, 0, BlockType::Engine),
            p(2, 0, 0, BlockType::Reactor),
        ];
        let r = validate(&ship);
        assert!(r.is_valid, "expected valid, problems: {:?}", r.problems);
        assert!(r.problems.is_empty());
    }

    #[test]
    fn empty_design_invalid() {
        let r = validate(&[]);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("empty")));
    }

    #[test]
    fn missing_command_core_invalid() {
        let ship = [p(0, 0, 0, BlockType::Engine), p(1, 0, 0, BlockType::Reactor)];
        let r = validate(&ship);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("command core")));
    }

    #[test]
    fn missing_engine_invalid() {
        let ship = [p(0, 0, 0, BlockType::CommandCore), p(1, 0, 0, BlockType::Reactor)];
        let r = validate(&ship);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("engine")));
    }

    #[test]
    fn insufficient_power_invalid() {
        // core(-3) + engine(-5) + weapon(-8) = -16, no reactor.
        let ship = [
            p(0, 0, 0, BlockType::CommandCore),
            p(1, 0, 0, BlockType::Engine),
            p(2, 0, 0, BlockType::Weapon),
        ];
        let r = validate(&ship);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("power")));
    }

    #[test]
    fn detached_block_invalid() {
        // Connected core+engine+reactor, plus a hull floating far away.
        let ship = [
            p(0, 0, 0, BlockType::CommandCore),
            p(1, 0, 0, BlockType::Engine),
            p(-1, 0, 0, BlockType::Reactor),
            p(9, 9, 9, BlockType::Hull),
        ];
        let r = validate(&ship);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("not connected")));
    }

    #[test]
    fn overlapping_cells_invalid() {
        let ship = [
            p(0, 0, 0, BlockType::CommandCore),
            p(0, 0, 0, BlockType::CommandCore),
            p(1, 0, 0, BlockType::Engine),
            p(2, 0, 0, BlockType::Reactor),
        ];
        let r = validate(&ship);
        assert!(!r.is_valid);
        assert!(r.problems.iter().any(|m| m.contains("same cell")));
    }

    #[test]
    fn build_ticks_rounds_up() {
        assert_eq!(build_ticks(0), 1);
        assert_eq!(build_ticks(50), 1);
        assert_eq!(build_ticks(51), 2);
        assert_eq!(build_ticks(185), 4);
    }
}
