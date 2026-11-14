use eframe::egui;
use std::path::PathBuf;

pub(crate) const MAX_RECENT: usize = 10;

pub(crate) fn recent_file() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        Some(PathBuf::from(x))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    }?;
    Some(base.join("rusted").join("recent.txt"))
}

pub(crate) fn load_recent() -> Vec<PathBuf> {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.lines().filter(|l| !l.is_empty()).map(PathBuf::from).take(MAX_RECENT).collect())
        .unwrap_or_default()
}

pub(crate) struct WindowState {
    pub(crate) pos: Option<egui::Pos2>,
    pub(crate) size: egui::Vec2,
    pub(crate) maximized: bool,
}

pub(crate) fn window_file() -> Option<PathBuf> {
    recent_file().map(|f| f.with_file_name("window.txt"))
}

pub(crate) fn load_light_theme() -> bool {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f.with_file_name("theme.txt")).ok())
        .is_some_and(|s| s.trim() == "light")
}

pub(crate) fn save_light_theme(light: bool) {
    let Some(f) = recent_file().map(|f| f.with_file_name("theme.txt")) else { return };
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(f, if light { "light" } else { "dark" });
}

pub(crate) fn theme_visuals(light: bool) -> egui::Visuals {
    if light { egui::Visuals::light() } else { egui::Visuals::dark() }
}

pub(crate) fn load_window() -> Option<WindowState> {
    let s = std::fs::read_to_string(window_file()?).ok()?;
    let v: Vec<f32> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
    if v.len() != 5 || v[2] < 200.0 || v[3] < 150.0 || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    // Reject positions that are clearly off any screen (e.g. minimized sentinel values).
    let pos = (v[0] > -10000.0 && v[1] > -10000.0 && v[0] < 20000.0 && v[1] < 20000.0)
        .then(|| egui::pos2(v[0], v[1]));
    Some(WindowState { pos, size: egui::vec2(v[2], v[3]), maximized: v[4] != 0.0 })
}

pub(crate) fn save_window(w: &WindowState) {
    let Some(f) = window_file() else { return };
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let p = w.pos.unwrap_or(egui::pos2(-99999.0, -99999.0));
    let _ = std::fs::write(
        f,
        format!("{} {} {} {} {}", p.x, p.y, w.size.x, w.size.y, w.maximized as u8),
    );
}
