use crate::app::*;
use crate::editor::*;
use crate::search::*;
use crate::settings::*;
use crate::toolbar::*;
use eframe::egui::{self, Color32, FontId, Key, Modifiers};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn test_app() -> App {
    let mut app = App {
        tabs: Vec::new(),
        active: 0,
        next_id: 0,
        recent: Vec::new(),
        font_size: DEFAULT_FONT_SIZE,
        browser: None,
        message: None,
        focus_editor: true,
        filter_history: Vec::new(),
        replace_history: Vec::new(),
        toolbar_icons: None,
        about_open: false,
        light_theme: false,
        about_icon: None,
        editor_selection: None,
        pending_edit: None,
        edit_menu_open: false,
        clipboard_status: Ok(false),
        #[cfg(windows)]
        launches: None,
        clipboard_test_status: Some(Ok(true)),
        window: WindowState {
            pos: None,
            size: egui::vec2(800.0, 600.0),
            maximized: false,
        },
        last_poll: Instant::now(),
        status: (1, 1, 1),
    };
    app.new_tab(None, "abcdefghij\nabcdefghij\nabcdefghij\nabcdefghij".into(), false, 0);
    app
}

#[test]
fn startup_arguments_support_tail_and_literal_switch_filename() {
    let parse = |args: &[&str]| crate::parse_args(args.iter().map(std::ffi::OsString::from));
    assert_eq!(parse(&[]), Ok((None, false)));
    assert_eq!(parse(&["file.txt"]), Ok((Some(PathBuf::from("file.txt")), false)));
    assert_eq!(parse(&["-t", "file.txt"]), Ok((Some(PathBuf::from("file.txt")), true)));
    assert_eq!(parse(&["file.txt", "-t"]), Ok((Some(PathBuf::from("file.txt")), true)));
    assert_eq!(parse(&["--", "-t"]), Ok((Some(PathBuf::from("-t")), false)));
    assert_eq!(parse(&["-t", "--", "-t"]), Ok((Some(PathBuf::from("-t")), true)));
    assert!(parse(&["-t"]).is_err());
    assert!(parse(&["first", "second"]).is_err());
}

#[test]
fn startup_tail_opens_file_and_tracks_appends() {
    let path = std::env::temp_dir().join(format!(
        "rusted-startup-tail-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, "first\n").unwrap();
    let mut app = test_app();
    app.tabs.clear();
    app.open_startup_file(path.clone(), false);
    assert!(!app.tabs[0].tail);
    app.tabs.clear();
    app.open_startup_file(path.clone(), true);
    assert_eq!(app.tabs[0].text, "first\n");
    assert!(app.tabs[0].tail);
    assert!(app.tabs[0].scroll_to_end);
    std::fs::write(&path, "first\nsecond\n").unwrap();
    app.update_filter(true);
    assert_eq!(app.tabs[0].text, "first\nsecond\n");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn startup_tail_reports_missing_file_without_enabling_tail() {
    let path = std::env::temp_dir().join(format!(
        "rusted-startup-missing-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let mut app = test_app();
    app.tabs.clear();
    app.open_startup_file(path, true);
    assert!(!app.tabs[0].tail);
    assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot reload ")));
    app.tabs.clear();
    app.open_startup_file(std::env::temp_dir(), true);
    assert!(app.tabs.is_empty());
    assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot open ")));
}

#[cfg(windows)]
#[test]
fn forwarded_launch_tails_existing_file() {
    let path = std::env::temp_dir().join(format!(
        "rusted-forward-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, "on disk").unwrap();
    let mut app = test_app();
    app.open_path(path.clone());
    app.tabs[app.active].text = "unsaved".into();
    app.tabs[app.active].dirty = true;
    let (sender, receiver) = std::sync::mpsc::channel();
    app.launches = Some(receiver);
    let launch = || crate::single_instance::Launch {
        file: Some(path.clone()), tail: true, error: None, focus_error: None,
    };
    sender.send(Ok(launch())).unwrap();
    app.handle_launches();
    assert_eq!(app.tabs[app.active].text, "on disk");
    assert!(app.tabs[app.active].tail);
    sender.send(Ok(launch())).unwrap();
    app.handle_launches();
    assert!(app.tabs[app.active].tail);
    let active = app.active;
    let count = app.tabs.len();
    sender.send(Ok(crate::single_instance::Launch {
        file: None, tail: false, error: Some("-t requires a file path".into()), focus_error: None,
    })).unwrap();
    app.handle_launches();
    assert_eq!(app.tabs.len(), count);
    assert_eq!(app.active, active);
    assert_eq!(app.message.as_deref(), Some("-t requires a file path"));
    std::fs::remove_file(path).unwrap();
}

#[derive(Debug)]
struct TestDrop(PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|error| error.to_string())
    }
}

#[test]
fn dropped_files_open_once_in_tabs_and_preserve_existing_edits() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "unsaved".into();
    app.tabs[0].dirty = true;
    app.browser = Some(Browser {
        mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
        name: String::new(), error: None, focus_name: false,
    });
    let dir = std::env::temp_dir();
    let id = format!("rusted-drop-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let first = dir.join(format!("{id}-1.txt"));
    let second = dir.join(format!("{id}-2.txt"));
    std::fs::write(&first, "first\r\nline").unwrap();
    std::fs::write(&second, "second").unwrap();
    let mut input = egui::RawInput::default();
    input.dropped_files = vec![std::sync::Arc::new(TestDrop(first.clone())), std::sync::Arc::new(TestDrop(second.clone()))];
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert!(input.dropped_files.is_empty());
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.tabs[0].text, "unsaved");
    assert_eq!(app.tabs[1].text, "first\nline");
    assert_eq!(app.tabs[2].text, "second");
    assert_eq!(app.active, 2);
    assert!(app.browser.is_none());
    assert_eq!(app.recent[0], second);
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert_eq!(app.tabs.len(), 3);
    app.tabs[1].text.push_str(" modified");
    app.tabs[1].dirty = true;
    input.dropped_files.push(std::sync::Arc::new(TestDrop(first.clone())));
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert_eq!(app.tabs.len(), 3);
    assert_eq!(app.active, 1);
    assert_eq!(app.tabs[1].text, "first\nline modified");
    std::fs::remove_file(first).unwrap();
    std::fs::remove_file(second).unwrap();
}

#[test]
fn dropped_invalid_paths_are_reported_without_hiding_successes() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let missing = std::env::temp_dir().join(format!(
        "rusted-drop-missing-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let valid = missing.with_extension("txt");
    std::fs::write(&valid, "valid").unwrap();
    let mut input = egui::RawInput::default();
    input.dropped_files = vec![
        std::sync::Arc::new(TestDrop(missing.clone())),
        std::sync::Arc::new(TestDrop(std::env::temp_dir())),
        std::sync::Arc::new(TestDrop(PathBuf::new())),
        std::sync::Arc::new(TestDrop(valid.clone())),
    ];
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert_eq!(app.tabs[app.active].text, "valid");
    let message = app.message.as_deref().unwrap();
    assert!(message.contains(&missing.to_string_lossy().to_string()));
    assert!(message.contains("not a regular file"));
    assert!(message.contains("no local path"));
    std::fs::remove_file(valid).unwrap();
}

#[test]
fn dropped_file_replaces_empty_untitled_tab() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text.clear();
    let path = std::env::temp_dir().join(format!(
        "rusted-drop-empty-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, "content").unwrap();
    let mut input = egui::RawInput::default();
    input.dropped_files.push(std::sync::Arc::new(TestDrop(path.clone())));
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].text, "content");
    std::fs::remove_file(path).unwrap();
}

fn frame(ctx: &egui::Context, app: &mut App, mut events: Vec<egui::Event>, alt: bool) -> egui::FullOutput {
    events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() }));
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.editor_ui(ui));
    output.textures_delta.clear();
    output
}

fn has_linear_selection(output: &egui::FullOutput, color: Color32) -> bool {
    output.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Text(text) => text.galley.rows.iter().any(|row| {
            row.visuals.mesh.vertices.iter().any(|vertex| vertex.color == color)
        }),
        _ => false,
    })
}

fn filter_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.filter_ui(ui));
    output.textures_delta.clear();
    output
}

#[test]
fn find_and_replace_inputs_are_left_aligned() {
    for size in [14.0, 24.0] {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| {
            style.text_styles.insert(egui::TextStyle::Body, FontId::proportional(size));
        });
        let mut app = test_app();
        app.tabs[0].filter_text = "find input".into();
        app.tabs[0].replace_text = "replacement input".into();
        let output = filter_frame(&ctx, &mut app, vec![]);
        let text_rect = |label: &str| output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some(text.galley.rect.translate(text.pos.to_vec2())),
            _ => None,
        }).unwrap();
        let find = text_rect("find input");
        let replace = text_rect("replacement input");
        let find_label = text_rect("Find:");
        let replace_label = text_rect("Replace:");
        assert!((find.left() - replace.left()).abs() < 0.1, "{find:?} {replace:?}");
        assert!((find_label.right() - replace_label.right()).abs() < 0.1, "{find_label:?} {replace_label:?}");
        assert!((find_label.center().y - find.center().y).abs() < 1.0, "{find_label:?} {find:?}");
        assert!((replace_label.center().y - replace.center().y).abs() < 1.0, "{replace_label:?} {replace:?}");
        assert!(find.top() < replace.top());
    }
}

#[test]
fn double_click_search_result_preserves_horizontal_scroll() {
    for font_size in [14.0, 30.0] {
        for horizontal in [0.0, 120.0] {
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
            let mut app = test_app();
            app.font_size = font_size;
            let long_line = "abcdefghij".repeat(80);
            app.tabs[0].text = (0..100).map(|i| {
                if i == 70 { format!("needle {long_line}") } else { format!("line {i} {long_line}") }
            }).collect::<Vec<_>>().join("\n");
            app.tabs[0].filter_open = true;
            app.tabs[0].filter_text = "needle".into();
            app.apply_search();
            app.tabs[0].highlight_color = HighlightColor::DarkRed;
            let mut time = 0.0;
            let mut run = |app: &mut App, events: Vec<egui::Event>| {
                time += 0.05;
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                };
                let mut offset = egui::Vec2::ZERO;
                let mut editor_pos = None;
                let mut output = ctx.run_ui(input, |ui| {
                    egui::Panel::bottom("test_filter").exact_size(150.0).show(ui, |ui| app.filter_ui(ui));
                    egui::CentralPanel::default().show(ui, |ui| {
                        editor_pos = Some(ui.min_rect().min);
                        app.editor_ui(ui);
                    });
                });
                if let Some(pos) = editor_pos {
                    let text = output.shapes.iter().find_map(|s| match &s.shape {
                        egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t),
                        _ => None,
                    }).unwrap();
                    offset = egui::vec2(text.pos.x - pos.x, text.pos.y - pos.y);
                }
                output.textures_delta.clear();
                (output, offset)
            };
            if horizontal > 0.0 {
                // Set the offset using the actual editor scroll-area ID in the same panel.
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        egui::ScrollArea::both().id_salt(("scroll", app.tabs[0].id))
                            .horizontal_scroll_offset(horizontal).show(ui, |ui| ui.set_min_size(egui::vec2(8000.0, 3000.0)));
                    });
                });
                output.textures_delta.clear();
            }
            let (output, before) = run(&mut app, vec![]);
            assert!(before.x.is_finite(), "{before:?}");
            if horizontal > 0.0 {
                assert!(before.x < -20.0, "horizontal scroll was not established: {before:?}");
            }
            let label = format!("{:>6}  needle {long_line}", 71);
            let pos = output.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.job.text == label => Some(t.pos + egui::vec2(60.0, t.galley.rect.center().y)),
                _ => None,
            }).unwrap();
            let button = |pressed| egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
            };
            run(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
            run(&mut app, vec![button(false)]);
            run(&mut app, vec![button(true)]);
            let (_, after) = run(&mut app, vec![button(false)]);
            let (_, settled) = run(&mut app, vec![]);
            let (output, settled2) = run(&mut app, vec![]);
            assert_eq!(app.tabs[0].highlight, Some(70));
            let text = output.shapes.iter().find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t),
                _ => None,
            }).unwrap();
            assert!(text.galley.job.sections.iter().any(|s| {
                s.format.color == Color32::YELLOW
                    && text.galley.job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)].starts_with("needle ")
            }));
            assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == Color32::from_rgba_unmultiplied(255, 214, 0, 55))));
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|shape| matches!(shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())))));
            assert!((after.x - before.x).abs() < 0.1, "{before:?} -> {after:?}");
            assert!((settled.x - before.x).abs() < 0.1, "{before:?} -> {settled:?}");
            assert!((settled2.x - before.x).abs() < 0.1, "{before:?} -> {settled2:?}");
            assert!(settled2.y < before.y - 100.0, "{before:?} -> {after:?} -> {settled:?} -> {settled2:?}");
        }
    }
}

#[test]
fn find_history_arrow_is_drawn_and_history_remains_selectable() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].filter_open = true;
    app.filter_history.push("history entry".into());
    let output = filter_frame(&ctx, &mut app, vec![]);
    let triangle = output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Path(path) if path.closed && path.points.len() == 3 => Some(path),
        _ => None,
    }).expect("find history arrow was not painted");
    let rect = egui::Rect::from_points(&triangle.points);
    assert!(rect.width() > 0.0 && rect.height() > 0.0);
    assert!(triangle.points.iter().any(|point| point.y > rect.center().y));
    let pos = rect.center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    let output = filter_frame(&ctx, &mut app, vec![]);
    let pos = find_label(&output, "history entry");
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    assert_eq!(app.tabs[0].filter_text, "history entry");
    assert_eq!(app.tabs[0].applied.0, "history entry");
}

#[test]
fn replace_history_tracks_recent_values_and_selection_does_not_replace() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].filter_open = true;
    app.tabs[0].filter_text = "not present".into();
    for i in 0..12 {
        app.tabs[0].replace_text = format!("replacement {i}");
        filter_frame(&ctx, &mut app, vec![]);
        ctx.run_ui(egui::RawInput::default(), |ui| app.replace_all(ui.ctx())).textures_delta.clear();
    }
    assert_eq!(app.replace_history.len(), 10);
    assert_eq!(app.replace_history[0], "replacement 11");
    assert_eq!(app.replace_history[9], "replacement 2");
    app.tabs[0].replace_text = "replacement 5".into();
    ctx.run_ui(egui::RawInput::default(), |ui| app.replace_all(ui.ctx())).textures_delta.clear();
    assert_eq!(app.replace_history[0], "replacement 5");
    assert_eq!(app.replace_history.iter().filter(|s| *s == "replacement 5").count(), 1);
    app.tabs[0].replace_text.clear();
    ctx.run_ui(egui::RawInput::default(), |ui| app.replace_all(ui.ctx())).textures_delta.clear();
    app.tabs[0].replace_text = "ignored".into();
    app.tabs[0].filter_text.clear();
    ctx.run_ui(egui::RawInput::default(), |ui| app.replace_all(ui.ctx())).textures_delta.clear();
    app.tabs[0].filter_text = "not present".into();
    app.tabs[0].tail = true;
    ctx.run_ui(egui::RawInput::default(), |ui| app.replace_all(ui.ctx())).textures_delta.clear();
    assert_eq!(app.replace_history.len(), 10);
    assert!(!app.replace_history.iter().any(|entry| entry == "ignored"));
    app.tabs[0].tail = false;
    app.filter_history.push("find history".into());
    app.new_tab(None, "replacement 5 not present".into(), false, 0);
    app.tabs[1].filter_open = true;
    let before = app.tabs[1].text.clone();
    let output = filter_frame(&ctx, &mut app, vec![]);
    let mut arrows: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Path(path) if path.closed && path.points.len() == 3 => Some(egui::Rect::from_points(&path.points).center()),
        _ => None,
    }).collect();
    arrows.retain(|p| p.x < find_label(&output, "Replace all").x);
    arrows.sort_by(|a, b| a.y.total_cmp(&b.y));
    assert_eq!(arrows.len(), 2);
    assert!((arrows[0].x - arrows[1].x).abs() < 0.1);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    let pos = arrows[1];
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    let output = filter_frame(&ctx, &mut app, vec![]);
    let pos = find_label(&output, "replacement 5");
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    assert_eq!(app.tabs[1].replace_text, "replacement 5");
    assert_eq!(app.tabs[1].text, before);
    assert!(!app.tabs[1].dirty);
}

#[test]
fn search_highlighting_tracks_color_searches_and_editing() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "alpha one\nbeta alpha\nother".into();
    app.tabs[0].filter_text = "alpha".into();
    app.apply_search();
    assert_eq!(app.tabs[0].highlight_color, HighlightColor::DarkGreen);
    assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 1, 0]);
    let output = frame(&ctx, &mut app, vec![], false);
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().filter(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color())).count() == 2)));
    app.tabs[0].highlight_color = HighlightColor::DarkRed;
    let output = frame(&ctx, &mut app, vec![], false);
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().filter(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())).count() == 2)));
    let text = output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text.contains("alpha one") => Some(t),
        _ => None,
    }).unwrap();
    assert!(text.galley.job.sections.iter().any(|s| s.format.color == Color32::WHITE && &text.galley.job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)] == "alpha"));
    app.tabs[0].highlight_color = HighlightColor::DarkGray;
    let output = frame(&ctx, &mut app, vec![], false);
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkRed.color())))));
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Vec(v) if v.iter().any(|x| matches!(x, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color())))));
    app.tabs[0].text = "alpha changed\nbeta alpha\nother".into();
    let tab = &mut app.tabs[0];
    tab.search_highlights.edited(&tab.text);
    assert!(app.tabs[0].search_highlights.lines[0].is_empty());
    assert!(!app.tabs[0].search_highlights.lines[1].is_empty());
    app.tabs[0].filter_text = "other".into();
    app.apply_search();
    assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 1]);
}

#[test]
fn search_highlighting_handles_unicode_wrap_and_tail_updates() {
    assert_eq!(match_ranges("İx İX", "i\u{307}x", false), vec![0..3, 4..7]);
    assert_eq!(match_ranges("Alpha alpha", "alpha", true), vec![6..11]);
    let mut highlights = SearchHighlights::default();
    highlights.search("hit\nother\nhit", "hit", false);
    highlights.edited("inserted\nhit\nother\nhit");
    assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 1, 0, 1]);
    highlights.edited("inserted\nmodified hit\nother\nhit");
    assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 0, 1]);
    highlights.tailed("inserted\nmodified hit\nother\nhit\nHIT", "hit", false);
    assert_eq!(highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![0, 0, 0, 1, 1]);
    let font = FontId::monospace(14.0);
    let job = search_layout("inserted\nmodified hit\nother\nhit\nHIT", &highlights, None, true, &font, 60.0, false);
    assert_eq!(job.wrap.max_width, 60.0);
    assert!(job.sections.iter().any(|s| s.format.color == Color32::WHITE && &job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)] == "HIT"));
    assert!(job.text.contains("modified·hit"));
}

#[test]
fn selected_search_line_uses_yellow_text_without_overriding_other_matches() {
    let mut highlights = SearchHighlights::default();
    let text = "hit first\nhit second\nother";
    highlights.search(text, "hit", false);
    for hidden in [false, true] {
        for width in [60.0, f32::INFINITY] {
            let job = search_layout(text, &highlights, Some(1), hidden, &FontId::monospace(14.0), width, false);
            let sections: Vec<_> = job.sections.iter().map(|s| (
                s.format.color,
                &job.text[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)],
            )).collect();
            assert!(sections.iter().any(|(color, text)| *color == Color32::WHITE && *text == "hit"));
            assert!(sections.iter().any(|(color, text)| *color == Color32::YELLOW && text.starts_with("hit")));
            assert!(!sections.iter().any(|(color, text)| *color == Color32::WHITE && text.contains("second")));
        }
    }
    highlights.active = false;
    let job = search_layout(text, &highlights, Some(1), false, &FontId::monospace(14.0), f32::INFINITY, false);
    assert!(job.sections.iter().any(|s| s.format.color == Color32::YELLOW));
}

#[test]
fn highlight_color_selector_changes_current_matches() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].filter_text = "abc".into();
    app.apply_search();
    let output = filter_frame(&ctx, &mut app, vec![]);
    let pos = find_label(&output, "Dark Green");
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkGreen.color() && r.rect.contains(pos))));
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    let output = filter_frame(&ctx, &mut app, vec![]);
    for color in HighlightColor::ALL {
        find_label(&output, color.name());
    }
    let pos = find_label(&output, "Dark Blue");
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    assert_eq!(app.tabs[0].highlight_color, HighlightColor::DarkBlue);
    assert_eq!(app.tabs[0].search_highlights.lines.len(), 4);
    let output = filter_frame(&ctx, &mut app, vec![]);
    let pos = find_label(&output, "Dark Blue");
    assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == HighlightColor::DarkBlue.color() && r.rect.contains(pos))));
    filter_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    filter_frame(&ctx, &mut app, vec![button(pos, false)]);
    let mut output = filter_frame(&ctx, &mut app, vec![]);
    for _ in 0..30 {
        output = filter_frame(&ctx, &mut app, vec![]);
    }
    for color in HighlightColor::ALL {
        let texts: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == color.name() => Some(t.pos + t.galley.rect.center().to_vec2()),
            _ => None,
        }).collect();
        assert_eq!(texts.len(), if color == HighlightColor::DarkBlue { 2 } else { 1 });
        for pos in texts {
            let fills: Vec<_> = output.shapes.iter().filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.rect.contains(pos) && r.fill != Color32::TRANSPARENT => Some(r.fill),
                _ => None,
            }).collect();
            assert_eq!(fills.last(), Some(&color.color()), "{}: {fills:?}", color.name());
        }
    }
}

#[test]
fn actual_editor_edits_clear_only_the_edited_line_highlight() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "hit\nhit\nhit".into();
    app.tabs[0].filter_text = "hit".into();
    app.apply_search();
    frame(&ctx, &mut app, vec![], false);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", app.tabs[0].id));
    let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut app, vec![egui::Event::Text("x".into())], false);
    assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 0, 1]);
}

#[test]
fn tail_poll_highlights_appended_matches() {
    let path = std::env::temp_dir().join(format!(
        "rusted-highlight-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, "hit\n").unwrap();
    let mut app = test_app();
    app.tabs[0].path = Some(path.clone());
    assert!(reload_from_disk(&mut app.tabs[0], &path));
    app.tabs[0].filter_open = true;
    app.tabs[0].filter_text = "hit".into();
    app.apply_search();
    app.tabs[0].tail = true;
    std::fs::write(&path, "hit\nmiss\nHIT\n").unwrap();
    app.update_filter(true);
    assert_eq!(app.tabs[0].search_highlights.lines.iter().map(|r| r.len()).collect::<Vec<_>>(), vec![1, 0, 1, 0]);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tail_scrolls_to_bottom_left_when_enabled_and_when_file_grows() {
    let path = std::env::temp_dir().join(format!(
        "rusted-tail-scroll-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let long = "abcdefghij".repeat(80);
    let initial = (0..100).map(|i| format!("line {i} {long}\n")).collect::<String>();
    std::fs::write(&path, &initial).unwrap();
    for font_size in [14.0, 30.0] {
        for horizontal in [0.0, 120.0] {
            let ctx = egui::Context::default();
            ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
            let mut app = test_app();
            app.font_size = font_size;
            app.tabs[0].path = Some(path.clone());
            assert!(reload_from_disk(&mut app.tabs[0], &path));
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::both().id_salt(("scroll", app.tabs[0].id))
                        .horizontal_scroll_offset(horizontal).show(ui, |ui| ui.set_min_size(egui::vec2(8000.0, 3000.0)));
                });
            });
            output.textures_delta.clear();
            let run = |app: &mut App| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
                    ..Default::default()
                };
                let mut area = egui::Rect::NOTHING;
                let mut output = ctx.run_ui(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        area = ui.clip_rect();
                        app.editor_ui(ui);
                    });
                });
                let text = output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text.starts_with("line 0 ") => Some(t),
                    _ => None,
                }).unwrap();
                let final_line = (app.tabs[0].text.bytes().filter(|&b| b == b'\n').count() + 1).to_string();
                let gutter = output.shapes.iter().find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.job.text == final_line => Some(t.pos.y),
                    _ => None,
                });
                let (x, bottom) = (text.pos.x, text.pos.y + text.galley.rect.bottom());
                output.textures_delta.clear();
                (x, bottom, area, gutter)
            };
            let (before_x, before_bottom, _, _) = run(&mut app);
            app.toggle_tail();
            assert!(app.tabs[0].tail);
            run(&mut app);
            let (after_x, after_bottom, area, gutter) = run(&mut app);
            if horizontal == 0.0 {
                assert!((after_x - before_x).abs() < 0.1, "{before_x} -> {after_x}");
            } else {
                assert!(after_x > before_x + 100.0, "{before_x} -> {after_x}");
            }
            assert!(before_bottom > area.bottom() + 100.0);
            assert!(gutter.is_some_and(|y| y >= area.bottom() - font_size * 3.0 && y <= area.bottom()), "{after_bottom} {area:?} {gutter:?}");
            let appended = format!("{initial}last {long}\n");
            std::fs::write(&path, &appended).unwrap();
            app.update_filter(true);
            assert!(app.tabs[0].scroll_to_end);
            run(&mut app);
            let (tail_x, tail_bottom, area, gutter) = run(&mut app);
            assert!((tail_x - after_x).abs() < 0.1);
            assert!(gutter.is_some_and(|y| y >= area.bottom() - font_size * 3.0 && y <= area.bottom()), "{tail_bottom} {area:?} {gutter:?}");
            std::fs::write(&path, &initial).unwrap();
        }
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tail_editor_stays_read_only_when_clicked_and_typed() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].tail = true;
    let output = frame(&ctx, &mut app, vec![], false);
    let text = output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.rows.len() == 4 => Some(t),
        _ => None,
    }).unwrap();
    let pos = text.pos + text.galley.rows[0].rect().center().to_vec2();
    let button = |pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos), button(true)], false);
    frame(&ctx, &mut app, vec![button(false), egui::Event::Text("x".into()), egui::Event::Paste("y".into())], false);
    assert!(app.tabs[0].tail);
    assert_eq!(app.tabs[0].text, "abcdefghij\nabcdefghij\nabcdefghij\nabcdefghij");
}

fn toolbar_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        app.menu_bar(ui);
        app.view_toolbar(ui);
        ui.separator();
        app.editor_ui(ui);
        app.about_ui(ui.ctx());
    });
    output.textures_delta.clear();
    output
}

fn find_label(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.job.text == label => {
            Some(text.pos + text.galley.rect.center().to_vec2())
        }
        _ => None,
    }).unwrap_or_else(|| panic!("missing {label}: {:?}", output.shapes.iter().filter_map(|s| match &s.shape {
        egui::Shape::Text(t) => Some(t.galley.job.text.clone()),
        _ => None,
    }).collect::<Vec<_>>()))
}

fn goto_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        app.goto_shortcut(ui.ctx());
        egui::Panel::top("test_menu").show(ui, |ui| app.menu_bar(ui));
        egui::Panel::bottom("test_status").show(ui, |ui| { ui.label("Status"); });
        if app.tabs[app.active].goto_open && app.browser.is_none() {
            egui::Panel::bottom(("goto", app.tabs[app.active].id)).show(ui, |ui| app.goto_ui(ui));
        }
        if app.tabs[app.active].filter_open && app.browser.is_none() {
            egui::Panel::bottom("test_goto_filter").exact_size(150.0).show(ui, |ui| app.filter_ui(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if app.browser.is_some() { app.browser_ui(ui); } else { app.editor_ui(ui); }
        });
    });
    output.textures_delta.clear();
    output
}

fn goto_click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) -> egui::FullOutput {
    let button = |pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    goto_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
    goto_frame(ctx, app, vec![button(false)]);
    goto_frame(ctx, app, vec![])
}

#[test]
fn replace_matches_handles_case_unicode_empty_and_nonoverlapping_matches() {
    assert_eq!(replace_matches("Foo foo\nfOO\n", "foo", "bar", false).as_deref(), Some("bar bar\nbar\n"));
    assert_eq!(replace_matches("Foo foo", "foo", "", true).as_deref(), Some("Foo "));
    assert_eq!(replace_matches("aaaa", "aa", "a", true).as_deref(), Some("aa"));
    assert_eq!(replace_matches("İ İX", "i\u{307}", "z", false).as_deref(), Some("z zX"));
    assert_eq!(replace_matches("İ", "\u{307}", "z", false).as_deref(), Some("z"));
    assert_eq!(replace_matches("abc", "", "x", false), None);
    assert_eq!(replace_matches("abc", "x", "y", false), None);
    assert_eq!(replace_matches("abc", "a", "a", true), None);
}

#[test]
fn replace_all_ui_updates_active_tab_results_and_supports_undo_redo() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs.clear();
    app.new_tab(Some(PathBuf::from("test.txt")), "foo FOO\nfoo\nother".into(), false, 0);
    app.new_tab(None, "foo other tab".into(), false, 0);
    app.active = 0;
    let output = goto_frame(&ctx, &mut app, vec![]);
    let output = goto_click(&ctx, &mut app, find_label(&output, "File"));
    let output = goto_click(&ctx, &mut app, find_label(&output, "Find / Replace"));
    assert!(app.tabs[0].filter_open);
    assert!(find_label(&output, "Replace:").y > find_label(&output, "Find:").y);
    app.tabs[0].filter_text = "foo".into();
    app.tabs[0].replace_text = "bar".into();
    app.apply_search();
    assert_eq!(app.tabs[0].filter.results.len(), 2);
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Replace all"));
    assert_eq!(app.tabs[0].text, "bar bar\nbar\nother");
    assert_eq!(app.tabs[1].text, "foo other tab");
    assert!(app.tabs[0].dirty);
    assert!(app.tabs[0].filter.results.is_empty());
    assert!(app.tabs[0].search_highlights.lines.iter().all(Vec::is_empty));
    goto_frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(app.tabs[0].text, "foo FOO\nfoo\nother");
    assert!(!app.tabs[0].dirty);
    app.update_filter(false);
    assert_eq!(app.tabs[0].filter.results.len(), 2);
    goto_frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(app.tabs[0].text, "bar bar\nbar\nother");
}

#[test]
fn replace_all_respects_tail_noops_case_and_per_tab_input() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "Foo foo".into();
    app.tabs[0].saved_text = app.tabs[0].text.clone();
    app.tabs[0].filter_open = true;
    app.tabs[0].filter_text = "foo".into();
    app.tabs[0].replace_text = String::new();
    app.tabs[0].case_sensitive = true;
    app.tabs[0].tail = true;
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Replace all"));
    assert_eq!(app.tabs[0].text, "Foo foo");
    assert!(app.tabs[0].tail);
    app.tabs[0].tail = false;
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Replace all"));
    assert_eq!(app.tabs[0].text, "Foo ");
    goto_frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(app.tabs[0].text, "Foo foo");
    app.tabs[0].filter_text.clear();
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Replace all"));
    app.focus_editor = true;
    goto_frame(&ctx, &mut app, vec![]);
    goto_frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(app.tabs[0].text, "Foo ");
    app.tabs[0].replace_text = "per tab".into();
    app.new_tab(None, "foo".into(), false, 0);
    assert_eq!(app.tabs[1].replace_text, "");
    assert_eq!(app.tabs[0].replace_text, "per tab");
}

#[test]
fn goto_line_shortcut_validation_cancel_and_tab_state() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let key = egui::Event::Key {
        key: Key::G, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND,
    };
    let output = goto_frame(&ctx, &mut app, vec![key.clone()]);
    assert!(app.tabs[0].goto_open);
    assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(("goto_input", app.tabs[0].id)))));
    assert!(find_label(&output, "Line:").y < find_label(&output, "Status").y);
    for invalid in ["", "0", "5", "1x", "-1", "999999999999999999999999999999999"] {
        app.tabs[0].goto_text = invalid.into();
        let output = goto_frame(&ctx, &mut app, vec![]);
        goto_click(&ctx, &mut app, find_label(&output, "Go"));
        assert!(app.tabs[0].goto_open, "{invalid:?}");
        assert_eq!(app.tabs[0].goto_line, None);
    }
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Cancel"));
    assert!(!app.tabs[0].goto_open);
    goto_frame(&ctx, &mut app, vec![key.clone()]);
    goto_frame(&ctx, &mut app, vec![egui::Event::Text("4".into())]);
    assert_eq!(app.tabs[0].goto_text, "4");
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Go"));
    assert!(!app.tabs[0].goto_open);
    assert_eq!(app.tabs[0].goto_line, Some(3));
    app.tabs[0].text = "first\n".into();
    goto_frame(&ctx, &mut app, vec![key.clone()]);
    goto_frame(&ctx, &mut app, vec![egui::Event::Text("2".into())]);
    let enter = egui::Event::Key {
        key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE,
    };
    goto_frame(&ctx, &mut app, vec![enter]);
    assert!(!app.tabs[0].goto_open);
    assert_eq!(app.tabs[0].goto_line, Some(1));
    app.new_tab(None, "other".into(), false, 0);
    assert_eq!(app.tabs[1].goto_line, None);
    app.open_browser();
    goto_frame(&ctx, &mut app, vec![key]);
    assert!(!app.tabs[1].goto_open);
}

#[test]
fn goto_line_menu_scrolls_and_paints_white_with_wrap_and_tail() {
    for wrap in [false, true] {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
        let mut app = test_app();
        app.tabs[0].text = (1..=100).map(|i| format!("line {i} {}", "abcdefghij".repeat(12)))
            .collect::<Vec<_>>().join("\n");
        app.tabs[0].word_wrap = wrap;
        app.tabs[0].tail = true;
        app.tabs[0].filter_open = true;
        let output = goto_frame(&ctx, &mut app, vec![]);
        let output = goto_click(&ctx, &mut app, find_label(&output, "File"));
        let output = goto_click(&ctx, &mut app, find_label(&output, "Go to line"));
        assert!(app.tabs[0].goto_open);
        assert!(ctx.memory(|m| m.has_focus(egui::Id::unique(("goto_input", app.tabs[0].id)))));
        app.tabs[0].goto_text = "71".into();
        goto_click(&ctx, &mut app, find_label(&output, "Go"));
        let output = goto_frame(&ctx, &mut app, vec![]);
        assert!(app.tabs[0].tail);
        assert_eq!(app.tabs[0].goto_line, Some(70));
        let number = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "71" => Some(t),
            _ => None,
        }).expect("target line number is not visible");
        assert_eq!(number.galley.job.sections[0].format.color, Color32::WHITE);
        assert!(number.pos.y >= 0.0 && number.pos.y < find_label(&output, "Status").y);
        let other = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.job.text == "70" => Some(t),
            _ => None,
        });
        if let Some(other) = other {
            assert_eq!(other.galley.job.sections[0].format.color, Color32::from_gray(110));
        }
    }
}

#[test]
fn goto_line_preserves_horizontal_scroll_and_search_highlight() {
    let ctx = egui::Context::default();
    ctx.all_styles_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    let mut app = test_app();
    app.tabs[0].text = (0..100).map(|i| format!("line {i} {}", "abcdefghij".repeat(80)))
        .collect::<Vec<_>>().join("\n");
    app.tabs[0].filter_text = "line".into();
    app.apply_search();
    app.tabs[0].highlight = Some(2);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        egui::Panel::top("test_menu").show(ui, |ui| app.menu_bar(ui));
        egui::Panel::bottom("test_status").show(ui, |ui| { ui.label("Status"); });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().id_salt(("scroll", app.tabs[0].id))
                .horizontal_scroll_offset(120.0).show(ui, |ui| ui.set_min_size(egui::vec2(8000.0, 3000.0)));
        });
    });
    output.textures_delta.clear();
    let output = goto_frame(&ctx, &mut app, vec![]);
    let text_x = |output: &egui::FullOutput| output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text.starts_with("line 0 ") => Some(t.pos.x),
        _ => None,
    }).unwrap();
    let before = text_x(&output);
    assert!(before < -20.0);
    app.open_goto();
    app.tabs[0].goto_text = "71".into();
    let output = goto_frame(&ctx, &mut app, vec![]);
    goto_click(&ctx, &mut app, find_label(&output, "Go"));
    let output = goto_frame(&ctx, &mut app, vec![]);
    assert!((text_x(&output) - before).abs() < 0.1);
    assert_eq!(app.tabs[0].highlight, Some(2));
    assert!(app.tabs[0].search_highlights.active);
}

fn tab_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        app.tab_bar(ui);
        ui.separator();
        app.editor_ui(ui);
    });
    output.textures_delta.clear();
    output
}

fn wrapped_tab_frame(ctx: &egui::Context, app: &mut App, width: f32, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        egui::Panel::top("tabs").show(ui, |ui| app.tab_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| app.editor_ui(ui));
    });
    output.textures_delta.clear();
    output
}

#[test]
fn tabs_wrap_and_remain_clickable_when_window_resizes() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs.clear();
    for i in 0..6 {
        app.new_tab(Some(PathBuf::from(format!("tab-{i}.txt"))), String::new(), false, 0);
    }
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![]);
    let output = wrapped_tab_frame(&ctx, &mut app, 240.0, vec![]);
    let first = find_label(&output, "tab-0.txt");
    let last = find_label(&output, "tab-5.txt");
    assert!(last.y > first.y, "tabs did not wrap: {first:?}, {last:?}");
    let fourth = find_label(&output, "tab-3.txt");
    assert!(fourth.y > first.y && (fourth.x - first.x).abs() < 1.0,
        "first overflowing tab did not start a new row: {first:?}, {fourth:?}");
    assert!((find_label(&output, "tab-2.txt").y - first.y).abs() < 1.0);
    assert!((last.y - fourth.y).abs() < 1.0);
    for i in 0..6 {
        let label = format!("tab-{i}.txt");
        let (text, clip) = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text == label => Some((text, shape.clip_rect)),
            _ => None,
        }).unwrap();
        let rect = text.galley.rect.translate(text.pos.to_vec2());
        assert!(rect.right() <= 240.0 && clip.contains_rect(rect),
            "tab {i} is clipped: {rect:?}, {clip:?}");
    }
    app.active = 0;
    let event = |pressed| egui::Event::PointerButton {
        pos: last, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE,
    };
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![egui::Event::PointerMoved(last), event(true)]);
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![event(false)]);
    assert_eq!(app.active, 5);

    let event = |pos, button, pressed| egui::Event::PointerButton {
        pos, button, pressed, modifiers: Modifiers::NONE,
    };
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![event(last, egui::PointerButton::Secondary, true)]);
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![event(last, egui::PointerButton::Secondary, false)]);
    let output = wrapped_tab_frame(&ctx, &mut app, 240.0, vec![]);
    let close = find_label(&output, "Close");
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![egui::Event::PointerMoved(close), event(close, egui::PointerButton::Primary, true)]);
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![event(close, egui::PointerButton::Primary, false)]);
    assert_eq!(app.tabs.len(), 5);
    assert!(app.tabs.iter().all(|tab| tab.title() != "tab-5.txt"));

    let output = wrapped_tab_frame(&ctx, &mut app, 800.0, vec![]);
    assert!((find_label(&output, "tab-4.txt").y - find_label(&output, "tab-0.txt").y).abs() < 1.0);
}

#[test]
fn long_tab_does_not_push_other_tabs_outside_window() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs.clear();
    let name = "a-very-long-file-name-that-does-not-fit-in-a-narrow-window.txt";
    app.new_tab(Some(PathBuf::from(name)), String::new(), false, 0);
    app.new_tab(Some(PathBuf::from("next.txt")), String::new(), false, 0);
    wrapped_tab_frame(&ctx, &mut app, 240.0, vec![]);
    let output = wrapped_tab_frame(&ctx, &mut app, 240.0, vec![]);
    let (rect, clip) = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.job.text == name =>
            Some((text.galley.rect.translate(text.pos.to_vec2()), shape.clip_rect)),
        _ => None,
    }).unwrap();
    assert!(rect.right() <= 240.0 && clip.contains_rect(rect), "long tab is clipped: {rect:?}, {clip:?}");
    let next = find_label(&output, "next.txt");
    assert!(next.x < 240.0 && next.y < 600.0, "next tab is outside the window: {next:?}");
}

fn tab_shortcut_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| {
        app.tab_shortcuts(ui.ctx());
        app.editor_ui(ui);
    });
    output.textures_delta.clear();
}

#[test]
fn tab_shortcuts_cycle_wrap_and_focus_active_editor() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "first".into();
    app.new_tab(None, "second".into(), false, 0);
    app.new_tab(None, "third".into(), false, 0);
    app.tabs[1].word_wrap = true;
    app.active = 0;
    tab_shortcut_frame(&ctx, &mut app, vec![]);
    let forward = || key_event(Key::Tab, Modifiers::CTRL);
    let backward = || key_event(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT);
    tab_shortcut_frame(&ctx, &mut app, vec![forward()]);
    assert_eq!(app.active, 1);
    assert!(app.tabs[1].word_wrap);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", app.tabs[1].id));
    assert!(ctx.memory(|memory| memory.has_focus(id)));
    assert_eq!(app.tabs[1].text, "second");
    tab_shortcut_frame(&ctx, &mut app, vec![forward(), forward()]);
    assert_eq!(app.active, 0);
    tab_shortcut_frame(&ctx, &mut app, vec![backward()]);
    assert_eq!(app.active, 2);
    tab_shortcut_frame(&ctx, &mut app, vec![backward(), forward()]);
    assert_eq!(app.active, 2);
}

#[test]
fn tab_shortcuts_preserve_plain_tab_and_handle_browser_and_single_tab() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "text".into();
    tab_shortcut_frame(&ctx, &mut app, vec![]);
    tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::CTRL)]);
    assert_eq!(app.tabs[0].text, "text");
    tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::NONE)]);
    assert!(app.tabs[0].text.contains('\t'));
    app.new_tab(None, "other".into(), false, 0);
    app.active = 0;
    app.browser = Some(Browser {
        mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
        name: String::new(), error: None, focus_name: false,
    });
    tab_shortcut_frame(&ctx, &mut app, vec![key_event(Key::Tab, Modifiers::CTRL | Modifiers::SHIFT)]);
    assert_eq!(app.active, 1);
    assert!(app.browser.is_none());
}

fn tab_click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2, button: egui::PointerButton) -> egui::FullOutput {
    let event = |pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE };
    tab_frame(ctx, app, vec![egui::Event::PointerMoved(pos), event(true)]);
    tab_frame(ctx, app, vec![event(false)])
}

fn tab_context(ctx: &egui::Context, app: &mut App, tab: &str, action: &str) -> egui::FullOutput {
    let output = tab_frame(ctx, app, vec![]);
    let pos = find_label(&output, tab);
    tab_click(ctx, app, pos, egui::PointerButton::Secondary);
    let output = tab_frame(ctx, app, vec![]);
    let pos = find_label(&output, action);
    tab_click(ctx, app, pos, egui::PointerButton::Primary)
}

#[test]
fn tab_unsaved_indicator_tracks_edits_undo_and_save() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let path = std::env::temp_dir().join(format!(
        "rusted-tabs-{}-{}.txt", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    app.tabs.clear();
    app.new_tab(Some(path.clone()), "original".into(), false, 0);
    let output = tab_frame(&ctx, &mut app, vec![]);
    find_label(&output, path.file_name().unwrap().to_str().unwrap());
    assert_eq!(app.tabs[0].tooltip(), path.to_string_lossy());
    tab_frame(&ctx, &mut app, vec![egui::Event::Text("x".into())]);
    assert!(app.tabs[0].dirty);
    let title = format!("{}*", path.file_name().unwrap().to_str().unwrap());
    find_label(&tab_frame(&ctx, &mut app, vec![]), &title);
    assert_eq!(app.tabs[0].tooltip(), format!("{} (unsaved)", path.display()));
    app.tabs[0].tail = true;
    assert_eq!(app.tabs[0].tooltip(), format!("Tailing {} (unsaved)", path.display()));
    app.tabs[0].tail = false;
    tab_frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)]);
    assert!(!app.tabs[0].dirty);
    assert_eq!(app.tabs[0].text, "original");
    tab_frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)]);
    assert!(app.tabs[0].dirty);
    app.write_tab(0, path.clone());
    assert!(!app.tabs[0].dirty);
    assert_eq!(app.tabs[0].tooltip(), path.to_string_lossy());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn tab_context_actions_target_clicked_tab() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs.clear();
    app.new_tab(Some(PathBuf::from("first.txt")), "first".into(), false, 0);
    app.new_tab(Some(PathBuf::from("second.txt")), "second".into(), false, 0);
    app.new_tab(Some(PathBuf::from("third.txt")), "third".into(), false, 0);
    app.tabs[0].dirty = true;
    app.tabs[2].dirty = true;
    let output = tab_context(&ctx, &mut app, "first.txt*", "Copy file path");
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "first.txt")));
    assert_eq!(app.active, 2);
    tab_context(&ctx, &mut app, "first.txt*", "Close all others");
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].text, "first");
    assert!(app.tabs[0].dirty);
    assert_eq!(app.active, 0);
    tab_context(&ctx, &mut app, "first.txt*", "Close");
    assert_eq!(app.tabs.len(), 1);
    assert!(app.tabs[0].path.is_none());
    assert!(app.tabs[0].text.is_empty());
}

#[test]
fn middle_click_closes_dirty_inactive_and_active_tabs() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs.clear();
    app.new_tab(Some(PathBuf::from("first.txt")), "first".into(), false, 0);
    app.new_tab(Some(PathBuf::from("second.txt")), "second".into(), false, 0);
    app.tabs[0].dirty = true;
    app.tabs[1].dirty = true;
    let output = tab_frame(&ctx, &mut app, vec![]);
    tab_click(&ctx, &mut app, find_label(&output, "first.txt*"), egui::PointerButton::Middle);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[app.active].text, "second");
    let output = tab_frame(&ctx, &mut app, vec![]);
    tab_click(&ctx, &mut app, find_label(&output, "second.txt*"), egui::PointerButton::Middle);
    assert_eq!(app.tabs.len(), 1);
    assert_eq!(app.tabs[0].tab_title(), "Untitled");
}

#[test]
fn untitled_tab_has_no_copy_path_and_keeps_plain_save_name() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].dirty = true;
    assert_eq!(app.tabs[0].tab_title(), "Untitled");
    assert_eq!(app.tabs[0].tooltip(), "Untitled (unsaved)");
    app.save_as();
    assert_eq!(app.browser.as_ref().unwrap().name, "Untitled");
    app.browser = None;
    let output = tab_context(&ctx, &mut app, "Untitled", "Copy file path");
    assert!(!output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))));
    assert_eq!(app.tabs.len(), 1);
}

#[test]
fn saved_file_named_untitled_shows_unsaved_marker() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].dirty = true;
    app.tabs[0].path = Some(PathBuf::from("Untitled"));
    let output = tab_frame(&ctx, &mut app, vec![]);
    find_label(&output, "Untitled*");
    assert_eq!(app.tabs[0].tooltip(), "Untitled (unsaved)");
}

fn open_edit(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    let output = toolbar_frame(ctx, app, vec![]);
    let pos = find_label(&output, "Edit");
    let button = |pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
    };
    toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
    toolbar_frame(ctx, app, vec![button(false)]);
    toolbar_frame(ctx, app, vec![])
}

fn click_edit(ctx: &egui::Context, app: &mut App, label: &str) -> egui::FullOutput {
    let output = open_edit(ctx, app);
    let pos = find_label(&output, label);
    let button = |pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
    };
    toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
    toolbar_frame(ctx, app, vec![button(false)]);
    toolbar_frame(ctx, app, vec![])
}

fn click_at(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) -> egui::FullOutput {
    let button = |pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
    };
    toolbar_frame(ctx, app, vec![egui::Event::PointerMoved(pos), button(true)]);
    toolbar_frame(ctx, app, vec![button(false)]);
    toolbar_frame(ctx, app, vec![])
}

#[test]
fn json_format_preserves_values_order_duplicates_and_escapes() {
    let input = r#" { "z":[1e400,{},[],true,null],"a":"a \"quoted\" : [x]", "z":123456789012345678901234567890 } "#;
    let expected = "{\n  \"z\": [\n    1e400,\n    {},\n    [],\n    true,\n    null\n  ],\n  \"a\": \"a \\\"quoted\\\" : [x]\",\n  \"z\": 123456789012345678901234567890\n}";
    assert_eq!(format_json(input).unwrap(), expected);
    assert_eq!(format_json(expected).unwrap(), expected);
    assert_eq!(format_json(" [ ] ").unwrap(), "[]");
    assert_eq!(format_json(r#""hello ☃""#).unwrap(), r#""hello ☃""#);
    for invalid in ["", "{\"a\":}", "[1,]", "true false", "{\"a\":1}\u{a0}"] {
        assert!(format_json(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn json_toolbar_formats_active_file_and_supports_undo_redo() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].path = Some(PathBuf::from("first.JSON"));
    app.tabs[0].text = r#"{"x":[1,2]}"#.into();
    app.tabs[0].filter_open = true;
    app.tabs[0].applied = ("2".into(), false);
    app.tabs[0].filter.applied = Some(("2".into(), false));
    app.new_tab(Some(PathBuf::from("other.txt")), "other".into(), false, 0);
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    assert!(!output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text == "Format JSON")));
    app.active = 0;
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    assert!(find_label(&output, "Tail").x < find_label(&output, "Format JSON").x);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    let formatted = "{\n  \"x\": [\n    1,\n    2\n  ]\n}";
    assert_eq!(app.tabs[0].text, formatted);
    assert!(app.tabs[0].dirty);
    assert!(app.tabs[0].filter.applied.is_none());
    assert_eq!(app.tabs[1].text, "other");
    app.update_filter(false);
    assert_eq!(app.tabs[0].filter.results[0].0, 3);
    click_edit(&ctx, &mut app, "Undo");
    assert_eq!(app.tabs[0].text, r#"{"x":[1,2]}"#);
    app.update_filter(false);
    assert_eq!(app.tabs[0].filter.results[0].0, 0);
    click_edit(&ctx, &mut app, "Redo");
    assert_eq!(app.tabs[0].text, formatted);
    app.update_filter(false);
    assert_eq!(app.tabs[0].filter.results[0].0, 3);
}

#[test]
fn json_toolbar_handles_invalid_json_and_disables_during_tail_or_browser() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].path = Some(PathBuf::from("test.json"));
    app.tabs[0].text = "{invalid}".into();
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    assert_eq!(app.tabs[0].text, "{invalid}");
    assert!(!app.tabs[0].dirty);
    assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot format JSON:")));
    app.tabs[0].text = r#"{"x":1}"#.into();
    app.tabs[0].tail = true;
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    assert_eq!(app.tabs[0].text, r#"{"x":1}"#);
    app.tabs[0].tail = false;
    app.browser = Some(Browser {
        mode: BrowserMode::Open, dir: PathBuf::new(), entries: Vec::new(),
        name: String::new(), error: None, focus_name: false,
    });
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    assert_eq!(app.tabs[0].text, r#"{"x":1}"#);
}

#[test]
fn json_format_noop_preserves_redo_and_changed_format_clears_redo() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].path = Some(PathBuf::from("test.json"));
    app.tabs[0].text = "{}".into();
    toolbar_frame(&ctx, &mut app, vec![]);
    toolbar_frame(&ctx, &mut app, vec![egui::Event::Text(" ".into())]);
    let edited = app.tabs[0].text.clone();
    click_edit(&ctx, &mut app, "Undo");
    assert_eq!(app.tabs[0].text, "{}");
    app.tabs[0].dirty = false;
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    assert!(!app.tabs[0].dirty);
    click_edit(&ctx, &mut app, "Redo");
    assert_eq!(app.tabs[0].text, edited);
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    assert_eq!(app.tabs[0].text, "{}");
    click_edit(&ctx, &mut app, "Undo");
    assert_eq!(app.tabs[0].text, edited);
    click_edit(&ctx, &mut app, "Undo");
    assert_eq!(app.tabs[0].text, "{}");
    toolbar_frame(&ctx, &mut app, vec![egui::Event::Text("  ".into())]);
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Format JSON"));
    click_edit(&ctx, &mut app, "Redo");
    assert_eq!(app.tabs[0].text, "{}");
}

#[test]
fn find_toolbar_toggles_active_tab_filter() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.new_tab(None, "second".into(), false, 0);
    app.active = 0;
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    let find = find_label(&output, "Find");
    assert!(find.x < find_label(&output, "Word wrap").x);
    let output = click_at(&ctx, &mut app, find);
    assert!(app.tabs[0].filter_open);
    assert!(app.tabs[0].focus_filter);
    assert!(!app.tabs[1].filter_open);
    click_at(&ctx, &mut app, find_label(&output, "Find"));
    assert!(!app.tabs[0].filter_open);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", app.tabs[0].id));
    assert!(ctx.memory(|memory| memory.has_focus(id)));
    app.browser = Some(Browser {
        mode: BrowserMode::Open,
        dir: PathBuf::new(),
        entries: Vec::new(),
        name: String::new(),
        error: None,
        focus_name: false,
    });
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    click_at(&ctx, &mut app, find_label(&output, "Find"));
    assert!(!app.tabs[0].filter_open);
}

#[test]
fn edit_menu_clipboard_actions_use_editor_selection() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text),
        _ => None,
    }).unwrap();
    let row = &text.galley.rows[0];
    let start = text.pos + egui::vec2(row.pos.x + row.glyphs[1].pos.x, row.rect().center().y);
    let end = text.pos + egui::vec2(row.pos.x + row.glyphs[4].pos.x, row.rect().center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
    };
    toolbar_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start), button(start, true)]);
    toolbar_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)]);
    toolbar_frame(&ctx, &mut app, vec![button(end, false)]);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", 0_u64));
    let range = egui::text_edit::TextEditState::load(&ctx, id).unwrap().cursor.char_range().unwrap();
    assert!(!range.is_empty(), "selection missing after drag: {range:?}");
    let output = click_edit(&ctx, &mut app, "Copy");
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd")), "{:?}", output.platform_output.commands);
    let output = click_edit(&ctx, &mut app, "Cut");
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd")));
    assert!(app.tabs[0].text.starts_with("aefghij"));
    let output = click_edit(&ctx, &mut app, "Paste");
    assert!(output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))));
    toolbar_frame(&ctx, &mut app, vec![egui::Event::Paste("XYZ".into())]);
    assert!(app.tabs[0].text.starts_with("aXYZefghij"));
}

#[test]
fn edit_menu_clipboard_respects_rectangular_selection_and_tail() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let output = toolbar_frame(&ctx, &mut app, vec![]);
    let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
        _ => None,
    }).unwrap();
    let row = &galley.rows[0];
    app.tabs[0].col_sel = Some(ColSel {
        anchor: (0, row.glyphs[1].pos.x),
        head: (1, row.glyphs[4].pos.x),
    });
    let output = click_edit(&ctx, &mut app, "Copy");
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd\nbcd")));
    let output = click_edit(&ctx, &mut app, "Cut");
    assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "bcd\nbcd")));
    assert!(app.tabs[0].text.starts_with("aefghij\naefghij"));
    app.tabs[0].tail = true;
    let text = app.tabs[0].text.clone();
    click_edit(&ctx, &mut app, "Cut");
    let output = click_edit(&ctx, &mut app, "Paste");
    assert_eq!(app.tabs[0].text, text);
    assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))));
    assert!(app.tabs[0].tail);
}

#[test]
fn edit_menu_undo_and_redo() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "original".into();
    toolbar_frame(&ctx, &mut app, vec![]);
    toolbar_frame(&ctx, &mut app, vec![egui::Event::Text("X".into())]);
    let edited = app.tabs[0].text.clone();
    assert_ne!(edited, "original");
    click_edit(&ctx, &mut app, "Undo");
    assert_eq!(app.tabs[0].text, "original");
    click_edit(&ctx, &mut app, "Redo");
    assert_eq!(app.tabs[0].text, edited);
}

fn key_event(key: Key, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn keyboard_undo_ignores_cursor_only_history_and_preserves_position() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    frame(&ctx, &mut app, vec![], false);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", app.tabs[0].id));
    let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
    let first = egui::text::CCursorRange::one(egui::text::CCursor::new(0));
    let last = egui::text::CCursorRange::one(egui::text::CCursor::new(app.tabs[0].text.len()));
    let mut undoer = egui::util::undoer::Undoer::default();
    undoer.add_undo(&(last, app.tabs[0].text.clone()));
    state.set_undoer(undoer);
    state.cursor.set_char_range(Some(first));
    state.store(&ctx, id);
    let before = frame(&ctx, &mut app, vec![], false);
    assert!(ctx.memory(|m| m.has_focus(id)), "editor lacks focus");
    assert_eq!(egui::text_edit::TextEditState::load(&ctx, id).unwrap().cursor.char_range(), Some(first), "before undo");
    let text_y = |output: &egui::FullOutput| output.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.rows.len() == 100 => Some(t.pos.y),
        _ => None,
    }).unwrap();
    let after = frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
    let state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
    assert_eq!(state.cursor.char_range(), Some(first));
    assert_eq!(text_y(&before), text_y(&after));
    assert_eq!(history_steps(&state, &app.tabs[0].text, false), 0);
}

#[test]
fn keyboard_undo_and_shift_redo_skip_cursor_only_history() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.tabs[0].text = "changed".into();
    frame(&ctx, &mut app, vec![], false);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", app.tabs[0].id));
    let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
    let at = |i| egui::text::CCursorRange::one(egui::text::CCursor::new(i));
    let mut undoer = egui::util::undoer::Undoer::default();
    undoer.add_undo(&(at(0), "original".into()));
    undoer.add_undo(&(at(1), "changed".into()));
    undoer.add_undo(&(at(2), "changed".into()));
    state.set_undoer(undoer);
    state.cursor.set_char_range(Some(at(2)));
    assert_eq!(history_steps(&state, "changed", false), 2);
    state.store(&ctx, id);
    assert!(ctx.memory(|m| m.has_focus(id)), "editor lacks focus");
    frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
    assert_eq!(app.tabs[0].text, "original");
    frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT)], false);
    assert_eq!(app.tabs[0].text, "changed");
    frame(&ctx, &mut app, vec![key_event(Key::Z, Modifiers::COMMAND)], false);
    assert_eq!(app.tabs[0].text, "original");
    frame(&ctx, &mut app, vec![key_event(Key::Y, Modifiers::COMMAND)], false);
    assert_eq!(app.tabs[0].text, "changed");
}

#[test]
fn edit_menu_disables_unavailable_actions_and_reports_clipboard_errors() {
    for (status, label) in [
        (Ok(false), "Paste"),
        (Err("test clipboard error".to_owned()), "Paste"),
        (Ok(true), "Undo"),
        (Ok(true), "Redo"),
        (Ok(true), "Cut"),
        (Ok(true), "Copy"),
    ] {
        let ctx = egui::Context::default();
        let mut app = test_app();
        app.clipboard_test_status = Some(status.clone());
        let before = app.tabs[0].text.clone();
        let output = click_edit(&ctx, &mut app, label);
        assert_eq!(app.tabs[0].text, before, "{label}");
        assert!(app.pending_edit.is_none(), "{label}");
        assert!(!output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_))), "{label}");
        assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::RequestPaste))), "{label}");
        if status.is_err() {
            let output = open_edit(&ctx, &mut app);
            assert!(output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains("test clipboard error"))));
        }
    }
}

#[test]
fn view_shortcuts_toggle_only_the_active_tab() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.new_tab(None, "second".into(), false, 0);
    app.active = 0;
    for key in [Key::W, Key::E, Key::T] {
        let input = egui::RawInput {
            events: vec![key_event(key, Modifiers::COMMAND)],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
        output.textures_delta.clear();
    }
    assert!(app.tabs[0].word_wrap && app.tabs[0].hidden_chars && app.tabs[0].tail);
    assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
    let input = egui::RawInput { events: vec![key_event(Key::T, Modifiers::COMMAND)], ..Default::default() };
    let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
    output.textures_delta.clear();
    assert!(!app.tabs[0].tail);
}

#[test]
fn view_shortcuts_respect_browser_and_failed_tail_reload() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    app.browser = Some(Browser {
        mode: BrowserMode::Open,
        dir: PathBuf::new(),
        entries: Vec::new(),
        name: String::new(),
        error: None,
        focus_name: false,
    });
    for key in [Key::W, Key::E, Key::T] {
        let input = egui::RawInput { events: vec![key_event(key, Modifiers::COMMAND)], ..Default::default() };
        let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
        output.textures_delta.clear();
    }
    assert!(!app.tabs[0].word_wrap && !app.tabs[0].hidden_chars && !app.tabs[0].tail);
    app.browser = None;
    app.tabs[0].path = Some(std::env::temp_dir().join(format!("rusted-missing-{}", std::process::id())).join("file"));
    let input = egui::RawInput { events: vec![key_event(Key::T, Modifiers::COMMAND)], ..Default::default() };
    let mut output = ctx.run_ui(input, |_| app.view_shortcuts(&ctx));
    output.textures_delta.clear();
    assert!(!app.tabs[0].tail);
    assert!(app.message.as_deref().is_some_and(|message| message.starts_with("Cannot reload ")));
}

#[test]
fn toolbar_controls_current_tab_and_tail_reloads() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    let path = std::env::temp_dir().join(format!(
        "rusted-toolbar-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::write(&path, "disk content").unwrap();
    app.tabs[0].path = Some(path.clone());
    app.tabs[0].text = "unsaved content".into();
    app.tabs[0].dirty = true;
    app.new_tab(None, "second tab".into(), false, 0);
    app.active = 0;

    let run = |app: &mut App, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.view_toolbar(ui));
        output.textures_delta.clear();
        output
    };
    for label in ["Word wrap", "Hidden characters", "Tail"] {
        let output = run(&mut app, vec![]);
        let pos = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.job.text.contains(label) => {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        }).unwrap();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        run(&mut app, vec![egui::Event::PointerMoved(pos), button(true)]);
        run(&mut app, vec![button(false)]);
    }
    assert!(app.tabs[0].word_wrap);
    assert!(app.tabs[0].hidden_chars);
    assert!(app.tabs[0].tail);
    assert_eq!(app.tabs[0].text, "disk content");
    assert!(!app.tabs[0].dirty);
    assert!(app.tabs[0].scroll_to_end);
    assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
    app.active = 1;
    run(&mut app, vec![]);
    assert!(!app.tabs[1].word_wrap && !app.tabs[1].hidden_chars && !app.tabs[1].tail);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hidden_tabs_keep_layout_and_draw_four_cell_markers() {
    for font_size in [14.0, 32.0, 60.0] {
        for scale in [1.0, 1.25] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            let mut app = test_app();
            app.font_size = font_size;
            app.tabs[0].text = "a\tb\tc\n\tdef".into();
            let normal = frame(&ctx, &mut app, vec![], false);
            app.tabs[0].hidden_chars = true;
            let hidden = frame(&ctx, &mut app, vec![], false);
            let get_galley = |output: &egui::FullOutput| {
                output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text.galley.clone()),
                    _ => None,
                }).unwrap()
            };
            let normal_galley = get_galley(&normal);
            let hidden_galley = get_galley(&hidden);
            for (normal_row, hidden_row) in normal_galley.rows.iter().zip(&hidden_galley.rows) {
                assert_eq!(normal_row.glyphs.len(), hidden_row.glyphs.len());
                for (normal, hidden) in normal_row.glyphs.iter().zip(&hidden_row.glyphs) {
                    assert_eq!(normal.chr, hidden.chr);
                    assert!((normal.pos.x - hidden.pos.x).abs() < 0.01);
                    assert!((normal.advance_width - hidden.advance_width).abs() < 0.01);
                }

            }
            let markers = hidden.shapes.iter().filter(|shape| match &shape.shape {
                egui::Shape::LineSegment { stroke, .. } => stroke.color == Color32::from_gray(140),
                _ => false,
            }).count();
            assert_eq!(markers, 18, "six strokes for each of three tabs at font size {font_size}");
        }
    }
}

#[test]
fn alt_drag_does_not_draw_linear_selection() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    frame(&ctx, &mut app, vec![], false);
    #[allow(deprecated)]
    let id = egui::Id::new(("editor", 0_u64));
    let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(0),
        egui::text::CCursor::new(25),
    )));
    state.store(&ctx, id);

    let color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill;
    let start = egui::pos2(75.0, 10.0);
    let end = egui::pos2(95.0, 55.0);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    let output = frame(&ctx, &mut app, vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
    ], true);
    assert!(app.tabs[0].col_drag);
    assert!(!has_linear_selection(&output, color));
    let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
    assert!(!has_linear_selection(&output, color));
    let rectangle_color = color;
    assert!(output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == rectangle_color)
    }));
    let selection = app.tabs[0].col_sel.unwrap();
    assert_ne!(selection.anchor.0, selection.head.0);
    assert_ne!(selection.anchor.1, selection.head.1);
    let output = frame(&ctx, &mut app, vec![
        egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
    ], true);
    assert!(!has_linear_selection(&output, color));
    assert!(!app.tabs[0].col_drag);
    assert!(ctx.memory(|m| m.has_focus(id)), "editor lost focus after drag");
    let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
        _ => None,
    }).unwrap();
    let copied = column_text(&app.tabs[0].text, &selection, &galley);
    let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == &copied)
    }));
    let original = app.tabs[0].text.clone();
    let output = frame(&ctx, &mut app, vec![egui::Event::Cut], false);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == &copied)
    }));
    let mut expected = original;
    delete_column(&mut expected, &selection, &galley);
    assert_eq!(app.tabs[0].text, expected);

    app.tabs[0].col_sel = Some(selection);
    let output = frame(&ctx, &mut app, vec![], false);
    let galley = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 4 => Some(text.galley.clone()),
        _ => None,
    }).unwrap();
    delete_column(&mut expected, &selection, &galley);
    frame(&ctx, &mut app, vec![egui::Event::Key {
        key: Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }], false);
    assert_eq!(app.tabs[0].text, expected);
}

#[test]
fn normal_mouse_drag_still_draws_linear_selection() {
    let ctx = egui::Context::default();
    let mut app = test_app();
    frame(&ctx, &mut app, vec![], false);
    let start = egui::pos2(75.0, 10.0);
    let end = egui::pos2(95.0, 55.0);
    let modifiers = egui::Modifiers::NONE;
    frame(&ctx, &mut app, vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers },
    ], false);
    let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], false);
    assert!(has_linear_selection(&output, ctx.style_of(ctx.theme()).visuals.selection.bg_fill));
    assert!(app.tabs[0].col_sel.is_none());
    frame(&ctx, &mut app, vec![
        egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers },
    ], false);
    let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if !text.is_empty())
    }), "ordinary selection was not copied");
}

#[test]
fn column_selection_copies_visible_characters_at_scaled_width() {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.25);
    let mut app = test_app();
    app.tabs[0].text = format!("{}\n{}", "0123456789".repeat(12), "abcdefghij".repeat(12));
    let output = frame(&ctx, &mut app, vec![], false);
    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text),
        _ => None,
    }).unwrap();
    let row0 = &text.galley.rows[0];
    let row1 = &text.galley.rows[1];
    let start = text.pos + egui::vec2(row0.pos.x + row0.glyphs[50].pos.x, row0.rect().center().y);
    let end = text.pos + egui::vec2(row1.pos.x + row1.glyphs[55].pos.x, row1.rect().center().y);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    frame(&ctx, &mut app, vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
    ], true);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
    frame(&ctx, &mut app, vec![
        egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
    ], true);
    let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == "01234\nabcde")
    }), "incorrect block copied: {:?}", output.platform_output.commands);
}

#[test]
fn column_selection_matches_glyph_positions_after_tab() {
    for scale in [1.0, 1.25, 1.5] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        let mut app = test_app();
        app.tabs[0].text = "prefix\tABCDEFG\nprefix\tABCDEFG".into();
        let output = frame(&ctx, &mut app, vec![], false);
        let text = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.rows.len() == 2 => Some(text),
            _ => None,
        }).unwrap();
        let row0 = &text.galley.rows[0];
        let row1 = &text.galley.rows[1];
        let start = text.pos + egui::vec2(row0.pos.x + row0.glyphs[8].pos.x, row0.rect().center().y);
        let end = text.pos + egui::vec2(row1.pos.x + row1.glyphs[11].pos.x, row1.rect().center().y);
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        frame(&ctx, &mut app, vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
        ], true);
        let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
        let selection_color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill;
        let rect = output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.fill == selection_color => Some(rect.rect),
            _ => None,
        }).unwrap();
        assert!((rect.left() - start.x).abs() < 0.01);
        assert!((rect.right() - end.x).abs() < 0.01);
        frame(&ctx, &mut app, vec![
            egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
        ], true);
        let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
        assert!(output.platform_output.commands.iter().any(|command| {
            matches!(command, egui::OutputCommand::CopyText(text) if text == "BCD\nBCD")
        }), "scale {scale}: {:?}", output.platform_output.commands);
    }
}

#[test]
fn column_selection_uses_visible_rows() {
    let ctx = egui::Context::default();
    ctx.set_pixels_per_point(1.25);
    let mut app = test_app();
    app.tabs[0].text = (0..100).map(|i| format!("{i:03}-abcdefgh")).collect::<Vec<_>>().join("\n");
    let output = frame(&ctx, &mut app, vec![], false);
    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 100 => Some(text),
        _ => None,
    }).unwrap();
    let start = text.pos + text.galley.rows[30].rect().center().to_vec2();
    let end = text.pos + text.galley.rows[32].rect().center().to_vec2() + egui::vec2(12.0, 0.0);
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    frame(&ctx, &mut app, vec![
        egui::Event::PointerMoved(start),
        egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: alt },
    ], true);
    let output = frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], true);
    let selection = app.tabs[0].col_sel.unwrap();
    assert_eq!((selection.anchor.0, selection.head.0), (30, 32));
    let text = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Text(text) if text.galley.rows.len() == 100 => Some(text),
        _ => None,
    }).unwrap();
    let selection_color = ctx.style_of(ctx.theme()).visuals.selection.bg_fill;
    let rect = output.shapes.iter().find_map(|shape| match &shape.shape {
        egui::Shape::Rect(rect) if rect.fill == selection_color => Some(rect.rect),
        _ => None,
    }).unwrap();
    assert!((rect.top() - (text.pos.y + text.galley.rows[30].rect().top())).abs() < 0.01);
    assert!((rect.bottom() - (text.pos.y + text.galley.rows[32].rect().bottom())).abs() < 0.01);
    frame(&ctx, &mut app, vec![
        egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: alt },
    ], true);
    let output = frame(&ctx, &mut app, vec![egui::Event::Copy], false);
    let (l0, l1, _, _) = selection.bounds();
    let expected = (l0..=l1)
        .map(|line| {
            let (c0, c1) = columns_for_row(&text.galley, line, selection.bounds().2, selection.bounds().3);
            format!("{line:03}-abcdefgh").chars().skip(c0).take(c1 - c0).collect::<String>()
        })
        .collect::<Vec<_>>().join("\n");
    assert!(!expected.is_empty());
    assert!(output.platform_output.commands.iter().any(|command| {
        matches!(command, egui::OutputCommand::CopyText(text) if text == &expected)
    }));
}
