use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 320.0])
            .with_title("Space4x"),
        ..Default::default()
    };
    eframe::run_native(
        "Space4x",
        options,
        Box::new(|_cc| Ok(Box::<ClientApp>::default())),
    )
}

#[derive(Default)]
struct ClientApp {}

impl eframe::App for ClientApp {
    // eframe 0.34: `ui` is the required method; the framework supplies the
    // root viewport's `Ui` (and `update` is deprecated).
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.heading("Hello, world!");
        ui.label("Space4x — eframe/egui scaffold (starframe-client).");
    }
}
