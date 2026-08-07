//! The Heightmap pane on its own, to look at its layout.
//!
//! The tool opens on the homepage grid, so a screenshot of this pane needs a
//! click first. This example draws the pane by itself instead, at a height
//! that shows every card at once.

use heightmap::gui::{SharedOptions, heightmap::HeightmapApp};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([760.0, 1300.0]),
        ..Default::default()
    };
    let mut app = HeightmapApp::default();
    let mut shared = SharedOptions::default();
    let mut installed = false;
    eframe::run_simple_native("heightmap pane", options, move |ctx, _frame| {
        if !installed {
            heightmap::gui::theme::install(ctx);
            installed = true;
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| app.draw(ui, &mut shared, false));
        });
    })
}
