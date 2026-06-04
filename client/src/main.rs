//! Starframe (Space4x) — desktop client (read-only).
//!
//! The client only READS server state (via SpacetimeDB subscriptions). Its one
//! command is to advance the simulation by N ticks: `advance_days(d)` (=
//! d × TICKS_PER_DAY) or `advance_ticks(n)`. The server records a `sim_run` row
//! when a batch completes; the subscription then streams the new state here.

mod module_bindings;
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
        Box::new(|_cc| Ok(Box::new(StarframeApp::new()))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Empire,
    Systems,
}

struct StarframeApp {
    conn: Option<DbConnection>,
    // Keep the subscription handle alive for the connection's lifetime.
    _sub: Option<SubscriptionHandle>,
    status: String,
    screen: Screen,
}

impl StarframeApp {
    fn new() -> Self {
        match Self::connect() {
            Ok(conn) => {
                let sub = conn
                    .subscription_builder()
                    .on_applied(|_ctx| {})
                    .on_error(|_ctx, err| eprintln!("subscription error: {err}"))
                    .subscribe_to_all_tables();
                Self {
                    conn: Some(conn),
                    _sub: Some(sub),
                    status: format!("connecting to {DB_NAME} @ {SERVER_URI}…"),
                    screen: Screen::Empire,
                }
            }
            Err(e) => Self {
                conn: None,
                _sub: None,
                status: format!("connection failed: {e}"),
                screen: Screen::Empire,
            },
        }
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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

        egui::ScrollArea::vertical().show(ui, |ui| match self.screen {
            Screen::Empire => self.empire_view(ui),
            Screen::Systems => self.systems_view(ui),
        });

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
}
