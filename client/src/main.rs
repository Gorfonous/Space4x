//! Starframe (Space4x) — desktop client (read-only).
//!
//! The client only READS server state (via SpacetimeDB subscriptions). Its one
//! command is to advance the simulation by N ticks: `advance_days(d)` (=
//! d × TICKS_PER_DAY) or `advance_ticks(n)`. The server records a `sim_run` row
//! when a batch completes; the subscription then streams the new state here.

mod module_bindings;
mod editor;
#[cfg(debug_assertions)]
mod dev;

use eframe::egui;
use spacetimedb_sdk::{DbContext, Table};
use starframe_shared::TICKS_PER_DAY;

use module_bindings::*;

const SERVER_URI: &str = "http://127.0.0.1:3000";
const DB_NAME: &str = "space4x";

fn main() -> eframe::Result {
    // Debug builds bring the backend up first (start host + publish module) so
    // `cargo run -p starframe-client` is the only command needed. Set
    // STARFRAME_AUTOSTART=0 to skip and manage the server yourself.
    #[cfg(debug_assertions)]
    dev::bootstrap();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("Space4x"),
        ..Default::default()
    };
    eframe::run_native(
        "Space4x",
        options,
        Box::new(|cc| Ok(Box::new(StarframeApp::new(cc)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Empire,
    Systems,
    Designer,
}

struct EditorState {
    viewport: editor::Viewport,
    camera: editor::OrbitCamera,
    selected: BlockType,
    rotation: u8,
    design_name: String,
}

struct StarframeApp {
    conn: Option<DbConnection>,
    // Keep the subscription handle alive for the connection's lifetime.
    _sub: Option<SubscriptionHandle>,
    status: String,
    screen: Screen,
    editor: Option<EditorState>,
}

impl StarframeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let editor = cc.wgpu_render_state.as_ref().map(|rs| EditorState {
            viewport: editor::Viewport::new(rs),
            camera: editor::OrbitCamera::new(),
            selected: BlockType::CommandCore,
            rotation: 0,
            design_name: "New Design".to_string(),
        });
        let (conn, sub, status) = match Self::connect() {
            Ok(conn) => {
                let sub = conn
                    .subscription_builder()
                    .on_applied(|_ctx| {})
                    .on_error(|_ctx, err| eprintln!("subscription error: {err}"))
                    .subscribe_to_all_tables();
                (
                    Some(conn),
                    Some(sub),
                    format!("connecting to {DB_NAME} @ {SERVER_URI}…"),
                )
            }
            Err(e) => (None, None, format!("connection failed: {e}")),
        };
        Self { conn, _sub: sub, status, screen: Screen::Empire, editor }
    }

    fn connect() -> spacetimedb_sdk::Result<DbConnection> {
        DbConnection::builder()
            .with_uri(SERVER_URI)
            .with_database_name(DB_NAME)
            .on_connect(|_conn, _identity, _token| {})
            .on_connect_error(|_ctx, err| eprintln!("connect error: {err}"))
            .build()
    }

    fn current_tick(&self) -> Option<u64> {
        let conn = self.conn.as_ref()?;
        conn.db.game_state().iter().next().map(|gs| gs.current_tick)
    }

    fn factions_sorted(&self) -> Vec<Faction> {
        let Some(conn) = self.conn.as_ref() else { return Vec::new() };
        let mut v: Vec<Faction> = conn.db.faction().iter().collect();
        v.sort_by_key(|f| f.id);
        v
    }

    fn latest_sim_run(&self) -> Option<SimRun> {
        let conn = self.conn.as_ref()?;
        conn.db.sim_run().iter().max_by_key(|r| r.run_id)
    }

    /// Send the only client command: advance time. Errors only mean the request
    /// couldn't be sent; the result streams back via the subscription.
    fn advance_days(&self, days: u64) {
        if let Some(conn) = &self.conn {
            if let Err(e) = conn.reducers.advance_days(days) {
                eprintln!("advance_days({days}) failed to send: {e}");
            }
        }
    }

    fn advance_ticks(&self, n: u64) {
        if let Some(conn) = &self.conn {
            if let Err(e) = conn.reducers.advance_ticks(n) {
                eprintln!("advance_ticks({n}) failed to send: {e}");
            }
        }
    }
}

impl eframe::App for StarframeApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // Drain queued WebSocket messages into the client cache each frame.
        if let Some(conn) = &self.conn {
            let _ = conn.frame_tick();
        }

        let connected = self.conn.as_ref().map(|c| c.is_active()).unwrap_or(false);

        // ── Header: title, status, screen tabs, and the one command ──────────
        ui.horizontal(|ui| {
            ui.heading("Space4x");
            ui.separator();
            ui.label(if connected { "● connected" } else { "○ offline" });
            if let Some(t) = self.current_tick() {
                ui.separator();
                ui.label(format!("tick {t}"));
            }
        });

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.screen, Screen::Empire, "Empire");
            ui.selectable_value(&mut self.screen, Screen::Systems, "Systems");
            ui.selectable_value(&mut self.screen, Screen::Designer, "Designer");
            ui.separator();
            // The client's only mutations: advance time.
            if ui
                .button(format!("Advance 1 Day ({TICKS_PER_DAY} ticks)"))
                .clicked()
            {
                self.advance_days(1);
            }
            if ui.button("Advance 7 Days").clicked() {
                self.advance_days(7);
            }
            if ui.button("Advance 1 Tick").clicked() {
                self.advance_ticks(1);
            }
        });

        ui.separator();

        match self.screen {
            Screen::Empire => {
                egui::ScrollArea::vertical().show(ui, |ui| self.empire_view(ui));
            }
            Screen::Systems => {
                egui::ScrollArea::vertical().show(ui, |ui| self.systems_view(ui));
            }
            Screen::Designer => self.designer_ui(ui, frame),
        }

        // The connection advances asynchronously; keep repainting so the UI
        // reflects streamed updates without user input.
        ui.ctx().request_repaint();
    }
}

impl StarframeApp {
    fn empire_view(&self, ui: &mut egui::Ui) {
        ui.heading("Empire Overview");
        if !self.status.is_empty() && self.current_tick().is_none() {
            ui.label(&self.status);
        }
        if let Some(run) = self.latest_sim_run() {
            ui.label(format!(
                "last run: {} tick(s), {} → {}",
                run.requested_ticks, run.from_tick, run.to_tick
            ));
        }
        ui.add_space(8.0);

        let factions = self.factions_sorted();
        if factions.is_empty() {
            ui.label("Waiting for faction data…");
            return;
        }
        egui::Grid::new("factions")
            .striped(true)
            .num_columns(5)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                for h in ["Faction", "Kind", "Minerals", "Energy", "Research"] {
                    ui.strong(h);
                }
                ui.end_row();
                for f in &factions {
                    ui.label(&f.name);
                    ui.label(if f.is_ai { "AI" } else { "You" });
                    ui.label(f.minerals.to_string());
                    ui.label(f.energy.to_string());
                    ui.label(f.research.to_string());
                    ui.end_row();
                }
            });
    }

    fn systems_view(&self, ui: &mut egui::Ui) {
        ui.heading("Systems");
        let Some(conn) = self.conn.as_ref() else {
            ui.label("Not connected.");
            return;
        };

        // faction id → name, for owner display
        let names: std::collections::HashMap<u64, String> = conn
            .db
            .faction()
            .iter()
            .map(|f| (f.id, f.name))
            .collect();

        let mut systems: Vec<StarSystem> = conn.db.star_system().iter().collect();
        systems.sort_by_key(|s| s.id);
        if systems.is_empty() {
            ui.label("Waiting for galaxy data…");
            return;
        }

        egui::Grid::new("systems")
            .striped(true)
            .num_columns(4)
            .spacing([24.0, 4.0])
            .show(ui, |ui| {
                for h in ["System", "Owner", "Planets", "Mineral/tick"] {
                    ui.strong(h);
                }
                ui.end_row();
                for s in &systems {
                    let planets: Vec<Planet> = conn
                        .db
                        .planet()
                        .iter()
                        .filter(|p| p.system_id == s.id)
                        .collect();
                    let minerals: i64 = planets.iter().map(|p| p.minerals_output).sum();
                    let owner = s
                        .owner_faction_id
                        .and_then(|id| names.get(&id).cloned())
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(&s.name);
                    ui.label(owner);
                    ui.label(planets.len().to_string());
                    ui.label(minerals.to_string());
                    ui.end_row();
                }
            });
    }

    fn player_faction_id(&self) -> Option<u64> {
        let conn = self.conn.as_ref()?;
        let mut v: Vec<Faction> = conn.db.faction().iter().filter(|f| !f.is_ai).collect();
        v.sort_by_key(|f| f.id);
        v.first().map(|f| f.id)
    }

    fn active_draft_id(&self, faction: u64) -> Option<u64> {
        let conn = self.conn.as_ref()?;
        conn.db
            .ship_design_draft()
            .iter()
            .filter(|d| d.faction_id == faction)
            .max_by_key(|d| d.id)
            .map(|d| d.id)
    }

    fn draft_blocks(&self, draft: u64) -> Vec<([i32; 3], BlockType)> {
        let Some(conn) = self.conn.as_ref() else { return Vec::new() };
        conn.db
            .ship_design_draft_block()
            .iter()
            .filter(|b| b.draft_id == draft)
            .map(|b| ([b.x, b.y, b.z], b.block_type))
            .collect()
    }

    fn designer_ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.editor.is_none() {
            ui.label("3D editor unavailable (no wgpu render state).");
            return;
        }
        let Some(rs) = frame.wgpu_render_state() else {
            ui.label("3D editor unavailable (no wgpu render state).");
            return;
        };
        let Some(faction) = self.player_faction_id() else {
            ui.label("Waiting for faction data…");
            return;
        };
        let draft_id = self.active_draft_id(faction);
        let blocks: Vec<([i32; 3], BlockType)> =
            draft_id.map(|d| self.draft_blocks(d)).unwrap_or_default();

        let placements: Vec<starframe_shared::BlockPlacement> = blocks
            .iter()
            .map(|(c, t)| starframe_shared::BlockPlacement::new(c[0], c[1], c[2], editor::to_shared(t)))
            .collect();
        let stats = starframe_shared::ship_stats(&placements);
        let report = starframe_shared::validate(&placements);

        egui::Panel::left("designer_left")
            .resizable(false)
            .show_inside(ui, |ui| {
                ui.heading("Ship Designer");
                let ed = self.editor.as_mut().unwrap();
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut ed.design_name);
                });
                if draft_id.is_none() {
                    ui.label("No active draft.");
                } else {
                    ui.label(format!("Editing draft #{}", draft_id.unwrap()));
                }
                if ui.button("New Draft").clicked() {
                    if let Some(conn) = &self.conn {
                        let _ = conn.reducers.create_draft(faction, ed.design_name.clone());
                    }
                }
                ui.separator();
                ui.label("Block palette:");
                for (variant, label) in [
                    (BlockType::CommandCore, "Command Core"),
                    (BlockType::Hull, "Hull"),
                    (BlockType::Engine, "Engine"),
                    (BlockType::Reactor, "Reactor"),
                    (BlockType::Weapon, "Weapon"),
                    (BlockType::Sensor, "Sensor"),
                ] {
                    if ui.selectable_label(ed.selected == variant, label).clicked() {
                        ed.selected = variant;
                    }
                }
                ui.separator();
                ui.label(format!("Rotation: {} (press R)", ed.rotation));
                ui.separator();
                ui.label(format!("Blocks: {}", stats.block_count));
                ui.label(format!("Mass: {:.1}", stats.mass));
                ui.label(format!("Cost: {}", stats.cost));
                ui.label(format!("HP: {}", stats.max_hp));
                ui.label(format!("Attack: {}", stats.attack));
                ui.label(format!("Power: {}", stats.power));
                ui.separator();
                if report.is_valid {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "✔ valid design");
                } else {
                    for p in &report.problems {
                        ui.colored_label(egui::Color32::from_rgb(220, 100, 100), format!("✖ {p}"));
                    }
                }
                let can_commit = draft_id.is_some() && report.is_valid;
                if ui
                    .add_enabled(can_commit, egui::Button::new("Commit Design"))
                    .clicked()
                {
                    if let (Some(d), Some(conn)) = (draft_id, &self.conn) {
                        let _ = conn.reducers.commit_design(d, ed.design_name.clone());
                    }
                }
                ui.add_space(8.0);
                ui.label("Drag: orbit · Scroll: zoom");
                ui.label("Click: place · Right-click: remove");
            });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let size = ui.available_size();
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
            let ed = self.editor.as_mut().unwrap();

            if response.dragged() {
                let d = response.drag_delta();
                ed.camera.orbit(d.x, d.y);
            }
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if response.hovered() && scroll != 0.0 {
                ed.camera.zoom(scroll * 0.5);
            }
            if ui.input(|i| i.key_pressed(egui::Key::R)) {
                ed.rotation = (ed.rotation + 1) % 4;
            }

            let aspect = rect.width() / rect.height().max(1.0);
            let view_proj = ed.camera.view_proj(aspect);

            let cells: Vec<[i32; 3]> = blocks.iter().map(|(c, _)| *c).collect();
            let mut ghost: Option<[i32; 3]> = None;
            if let Some(pos) = response.hover_pos() {
                let uv = (
                    ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0),
                    ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0),
                );
                if let Some(p) = editor::pick(view_proj, uv, &cells) {
                    ghost = Some(p.ghost);
                    if response.clicked() {
                        if let (Some(d), Some(conn)) = (draft_id, &self.conn) {
                            let _ = conn.reducers.place_block(
                                d,
                                p.ghost[0],
                                p.ghost[1],
                                p.ghost[2],
                                ed.selected.clone(),
                                ed.rotation,
                            );
                        }
                    }
                    if response.secondary_clicked() {
                        if let (Some(d), Some(conn), Some(rm)) = (draft_id, &self.conn, p.remove) {
                            let _ = conn.reducers.remove_block(d, rm[0], rm[1], rm[2]);
                        }
                    }
                }
            }

            let mut instances: Vec<editor::Instance> = Vec::new();
            instances.push(editor::Instance {
                offset: [0.0, -0.55, 0.0],
                scale: [40.0, 0.1, 40.0],
                color: [0.10, 0.11, 0.14, 1.0],
            });
            for (c, t) in &blocks {
                instances.push(editor::Instance {
                    offset: [c[0] as f32, c[1] as f32, c[2] as f32],
                    scale: [0.92, 0.92, 0.92],
                    color: editor::block_color(t),
                });
            }
            if let Some(g) = ghost {
                let mut col = editor::block_color(&ed.selected);
                col[3] = 0.4;
                instances.push(editor::Instance {
                    offset: [g[0] as f32, g[1] as f32, g[2] as f32],
                    scale: [0.96, 0.96, 0.96],
                    color: col,
                });
            }

            let ppp = ui.ctx().pixels_per_point();
            let px = (
                (rect.width() * ppp).max(1.0) as u32,
                (rect.height() * ppp).max(1.0) as u32,
            );
            let tex = ed.viewport.render(rs, px, &instances, view_proj);
            ui.painter().image(
                tex,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        });
    }
}
