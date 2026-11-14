#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod search;
mod settings;
mod toolbar;
#[cfg(test)]
mod tests;

use app::*;
use eframe::egui;
use settings::*;
use std::path::PathBuf;

pub(crate) fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let icon = eframe::icon_data::from_png_bytes(APP_ICON).expect("Cannot load application icon");
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]).with_title("RustEd").with_icon(icon).with_drag_and_drop(true);
    let mut centered = true;
    if let Some(w) = load_window() {
        viewport = viewport.with_inner_size(w.size).with_maximized(w.maximized);
        if let Some(p) = w.pos {
            viewport = viewport.with_position(p);
            centered = false;
        }
    }
    let options = eframe::NativeOptions {
        viewport,
        centered,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native("rusted", options, Box::new(|cc| Ok(Box::new(App::new(cc, file)))))
}
