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

## Workspace layout (planned)

```text
starframe/
├── shared/   # starframe-shared — enums, formulas, pure sim algorithms
├── server/   # starframe-module — SpacetimeDB tables + reducers (wasm)
└── client/   # starframe-client — eframe/egui/wgpu app
```

## Status

Pre‑implementation. The TDD is the source of truth; code lands per the milestone plan (M1 Core Engine → M2 Client Foundation → M3 Ship Editor → M4 Simulation Loop).
