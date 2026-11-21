#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod document;
mod editor;
mod search;
mod settings;
mod toolbar;
mod virtual_preview;
mod wrapped_preview;
mod sparse_heights;
mod virtual_filter;
#[cfg(windows)]
mod single_instance;
#[cfg(test)]
mod tests;

use app::*;
use eframe::egui;
use settings::*;
use std::ffi::OsString;
use std::path::PathBuf;

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<(Option<PathBuf>, bool), &'static str> {
    let mut file = None;
    let mut tail = false;
    for arg in args {
        if arg == "-t" {
            tail = true;
        } else if file.is_none() {
            file = Some(PathBuf::from(arg));
        } else {
            return Err("Only one file may be specified");
        }
    }
    if tail && file.is_none() {
        return Err("-t requires a file path");
    }
    Ok((file, tail))
}

pub(crate) fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let (_instance_lock, listener) = match single_instance::Instance::acquire() {
        Ok(single_instance::Instance::Primary { lock, listener }) => (lock, listener),
        Ok(single_instance::Instance::Secondary { name }) => {
            if let Err(error) = single_instance::forward(&name, std::env::args_os().skip(1)) {
                single_instance::report_error(&format!("Cannot contact the running RustEd instance: {error}"));
                return Err(error.into());
            }
            return Ok(());
        }
        Err(error) => {
            single_instance::report_error(&format!("Cannot start RustEd: {error}"));
            return Err(error.into());
        }
    };
    let (file, tail) = parse_args(std::env::args_os().skip(1))
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
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
    eframe::run_native("rusted", options, Box::new(move |cc| {
        let app = App::new(cc, file, tail);
        #[cfg(windows)]
        let app = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let RawWindowHandle::Win32(window) = cc.window_handle()?.as_raw() else {
                return Err("Cannot obtain the Windows window handle".into());
            };
            let mut app = app;
            app.launches = Some(single_instance::listen(listener, cc.egui_ctx.clone(), Some(window.hwnd.get())));
            app
        };
        Ok(Box::new(app))
    }))?;
    Ok(())
}
