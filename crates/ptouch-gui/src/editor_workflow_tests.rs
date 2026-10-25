// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

use super::*;
use crate::document_session::SaveKind;
use crate::file_workflow::{self, ConfirmChoice, FileAction};
use crate::state::{CanvasEditTarget, LabelElement};
use ptouch_render::document::{LabelDocument, LabelObject, TextBox};

fn app() -> PtouchApp {
    let (_tx, rx) = mpsc::channel();
    PtouchApp {
        state: AppState::default(),
        renderer: TextRenderer::new(),
        resp_rx: rx,
        smoke_frames: None,
    }
}

fn frame(
    ctx: &egui::Context,
    app: &mut PtouchApp,
    size: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(ctx.input(|input| input.time) + 0.05),
            events,
            ..Default::default()
        },
        |ui| app.show_editor(ui),
    );
    output.textures_delta.clear();
    output
}

fn text_rect(output: &egui::FullOutput, expected: &str) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == expected => {
                Some(text.galley.rect.translate(text.pos.to_vec2()))
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "missing {expected}: {:?}",
                output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            )
        })
}

fn click(pos: egui::Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        pointer(pos, true),
        pointer(pos, false),
    ]
}

fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key, command: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers {
            ctrl: command,
            command,
            ..Default::default()
        },
    }
}

#[test]
fn save_shortcut_commits_the_pending_numeric_value_before_writing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pending.ptl");
    let mut app = app();
    app.state.selected_element = Some(0);
    app.state.canvas_edit_target = CanvasEditTarget::Slot;
    app.state.elements = vec![LabelElement::Object(LabelObject::text(
        "PORT",
        TextBox {
            width_mm: 14.5,
            ..Default::default()
        },
    ))];
    assert!(file_workflow::save_to(
        &mut app.state,
        SaveKind::SaveAs,
        Some(&path)
    ));
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 900.0);
    frame(&ctx, &mut app, size, vec![]);
    let output = frame(&ctx, &mut app, size, vec![]);
    let field = text_rect(&output, "14.5").center();
    frame(&ctx, &mut app, size, click(field));
    frame(&ctx, &mut app, size, vec![]);
    frame(&ctx, &mut app, size, vec![key(egui::Key::A, true)]);
    frame(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::Text("18.75".into())],
    );
    assert_eq!(
        app.state.elements[0].as_object().unwrap().frame.width_mm,
        14.5
    );
    frame(&ctx, &mut app, size, vec![key(egui::Key::S, true)]);
    let saved = LabelDocument::from_toml_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(saved.elements[0].as_object().unwrap().frame.width_mm, 18.75);
    assert!(
        !app.state
            .document_session
            .is_modified(&app.state.document())
    );
}

#[test]
fn new_confirmation_cancel_and_discard_preserve_the_expected_document() {
    let mut app = app();
    app.state.add_text();
    let ctx = egui::Context::default();
    let size = egui::vec2(1000.0, 700.0);
    frame(&ctx, &mut app, size, vec![]);
    let original = app.state.document();
    file_workflow::request(&mut app.state, FileAction::New);
    frame(&ctx, &mut app, size, vec![]);
    let output = frame(&ctx, &mut app, size, vec![]);
    let cancel = text_rect(&output, "Cancel").center();
    frame(
        &ctx,
        &mut app,
        size,
        vec![egui::Event::PointerMoved(cancel), pointer(cancel, true)],
    );
    frame(&ctx, &mut app, size, vec![pointer(cancel, false)]);
    assert_eq!(app.state.document(), original);
    assert_eq!(app.state.pending_file_action, None);
    file_workflow::request(&mut app.state, FileAction::New);
    let output = frame(&ctx, &mut app, size, vec![]);
    let discard = text_rect(&output, "Discard").center();
    frame(&ctx, &mut app, size, click(discard));
    assert!(app.state.elements.is_empty());
    assert_eq!(app.state.document_session.current_path(), None);
    assert!(
        !app.state
            .document_session
            .is_modified(&app.state.document())
    );
    assert!(!app.state.history.can_undo());
}

#[test]
fn failed_save_keeps_the_pending_close_and_unsaved_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("protected.ptl");
    let mut state = AppState::default();
    state.add_text();
    assert!(file_workflow::save_to(
        &mut state,
        SaveKind::SaveAs,
        Some(&path)
    ));
    let original = std::fs::read(&path).unwrap();
    let permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut protected = permissions.clone();
    protected.set_readonly(true);
    std::fs::set_permissions(&path, protected).unwrap();
    state.add_text();
    let edited = state.document();
    file_workflow::request(&mut state, FileAction::Close);
    assert_eq!(
        file_workflow::apply_choice(&mut state, ConfirmChoice::Save),
        None
    );
    std::fs::set_permissions(&path, permissions).unwrap();
    assert_eq!(state.pending_file_action, Some(FileAction::Close));
    assert_eq!(state.document(), edited);
    assert!(state.document_session.is_modified(&edited));
    assert_eq!(std::fs::read(path).unwrap(), original);
    file_workflow::apply_choice(&mut state, ConfirmChoice::Cancel);
    assert_eq!(state.pending_file_action, None);
    assert!(!state.close_approved);
}

#[test]
fn invalid_open_and_cancel_leave_the_current_session_in_place() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("working.ptl");
    let mut state = AppState::default();
    state.add_text();
    assert!(file_workflow::save_to(
        &mut state,
        SaveKind::SaveAs,
        Some(&path)
    ));
    let original = state.document();
    let bad = directory.path().join("invalid.ptl");
    std::fs::write(&bad, "invalid layout").unwrap();
    assert!(file_workflow::open_path(&mut state, &bad).is_err());
    assert_eq!(state.document(), original);
    assert_eq!(state.document_session.current_path(), Some(path.as_path()));
    assert!(!state.document_session.is_modified(&original));
    file_workflow::request(&mut state, FileAction::Open);
    file_workflow::apply_choice(&mut state, ConfirmChoice::Cancel);
    assert_eq!(state.document(), original);
    assert_eq!(state.pending_file_action, None);
}

#[test]
fn title_and_success_status_show_the_filename_and_keep_the_path_in_the_tooltip() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sample.ptl");
    let mut state = AppState::default();
    state.add_text();
    assert!(file_workflow::save_to(
        &mut state,
        SaveKind::SaveAs,
        Some(&path)
    ));
    assert_eq!(state.status_message, "Saved sample.ptl");
    assert_eq!(
        state.status_tooltip.as_ref().unwrap().1,
        path.display().to_string()
    );
    assert_eq!(
        file_workflow::window_title(&state),
        "P-Touch Label Printer: sample.ptl"
    );
    state.add_text();
    assert_eq!(
        file_workflow::window_title(&state),
        "P-Touch Label Printer: sample.ptl (unsaved)"
    );
    assert!(file_workflow::open_path(&mut state, &path).is_ok());
    assert_eq!(state.status_message, "Opened sample.ptl");
    assert!(!file_workflow::window_title(&state).contains(directory.path().to_str().unwrap()));
}

#[test]
fn long_lists_keep_actions_visible_and_keyboard_navigation_does_not_move_objects() {
    for layers in [false, true] {
        let mut app = app();
        app.state.elements = (0..80)
            .map(|i| {
                LabelElement::Object(LabelObject::text(
                    format!("PORT{i:03}"),
                    TextBox {
                        x_mm: i as f64 * 12.0,
                        ..Default::default()
                    },
                ))
            })
            .collect();
        app.state.layers_view = layers;
        let ctx = egui::Context::default();
        let size = egui::vec2(800.0, 500.0);
        let output = frame(&ctx, &mut app, size, vec![]);
        for name in ["Duplicate", "Delete"] {
            let rect = text_rect(&output, name);
            assert!(
                rect.min.y >= 0.0 && rect.max.y <= size.y,
                "{name}: {rect:?}"
            );
        }
        crate::editor_navigation::set_focus(&ctx, crate::editor_navigation::EditorFocus::Elements);
        let document = app.state.document();
        frame(&ctx, &mut app, size, vec![key(egui::Key::ArrowDown, false)]);
        let first = app.state.selected_element;
        frame(&ctx, &mut app, size, vec![key(egui::Key::ArrowDown, false)]);
        assert_ne!(app.state.selected_element, first);
        assert_eq!(app.state.document(), document);
        if layers {
            assert!(text_rect(&output, "Front").max.y <= size.y);
        } else {
            assert!(text_rect(&output, "Up").max.y <= size.y);
            assert!(text_rect(&output, "Down").max.y <= size.y);
        }
    }
}

fn canvas_frame(
    ctx: &egui::Context,
    state: &mut AppState,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| crate::panels::canvas::show_canvas(ui, state),
    );
    output.textures_delta.clear();
    output
}

fn image_rect(output: &egui::FullOutput, id: egui::TextureId) -> egui::Rect {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id == id => Some(mesh.calc_bounds()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn ctrl_wheel_zooms_around_the_pointer_and_is_blocked_during_a_press() {
    let ctx = egui::Context::default();
    let mut state = AppState {
        zoom: 6.0,
        zoom_fit: false,
        needs_rerender: false,
        ..Default::default()
    };
    state.preview_bitmap = Some(ptouch_render::bitmap::LabelBitmap::new(2000, 76));
    let texture = ctx.load_texture(
        "zoom-test",
        egui::ColorImage::filled([2000, 76], egui::Color32::WHITE),
        Default::default(),
    );
    let id = texture.id();
    state.preview_texture = Some(texture);
    let output = canvas_frame(&ctx, &mut state, vec![]);
    let image = image_rect(&output, id);
    let point = image.min + egui::vec2(240.0, 120.0);
    let expected = (point - image.min) / state.zoom;
    let wheel = || egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        phase: egui::TouchPhase::Move,
        delta: egui::vec2(0.0, 40.0),
        modifiers: egui::Modifiers::CTRL,
    };
    let output = canvas_frame(
        &ctx,
        &mut state,
        vec![egui::Event::PointerMoved(point), wheel()],
    );
    assert!(state.zoom > 6.0);
    let after = image_rect(&output, id);
    let screen_error = ((point - after.min) - expected * state.zoom).length();
    assert!(
        screen_error <= 1.0 / ctx.pixels_per_point() + 0.001,
        "screen error {screen_error}"
    );
    canvas_frame(&ctx, &mut state, vec![pointer(point, true)]);
    let held_zoom = state.zoom;
    canvas_frame(&ctx, &mut state, vec![wheel()]);
    assert_eq!(state.zoom, held_zoom);
    canvas_frame(&ctx, &mut state, vec![pointer(point, false)]);
    for _ in 0..20 {
        canvas_frame(&ctx, &mut state, vec![]);
        assert_eq!(state.zoom, held_zoom);
    }
    canvas_frame(
        &ctx,
        &mut state,
        vec![pointer(point, true), wheel(), pointer(point, false)],
    );
    assert_eq!(state.zoom, held_zoom);
    for _ in 0..5 {
        canvas_frame(&ctx, &mut state, vec![]);
        assert_eq!(state.zoom, held_zoom);
    }
    let before = state.zoom;
    canvas_frame(
        &ctx,
        &mut state,
        vec![
            egui::Event::PointerMoved(egui::pos2(10.0, 10.0)),
            egui::Event::Zoom(1.5),
        ],
    );
    assert_eq!(state.zoom, before);
    canvas_frame(
        &ctx,
        &mut state,
        vec![egui::Event::PointerMoved(egui::pos2(10.0, 10.0)), wheel()],
    );
    for _ in 0..5 {
        canvas_frame(&ctx, &mut state, vec![egui::Event::PointerMoved(point)]);
        assert_eq!(state.zoom, before);
    }
    canvas_frame(&ctx, &mut state, vec![egui::Event::Zoom(1.25)]);
    assert!(state.zoom > before);
}

#[test]
fn selected_rows_scroll_into_view_without_changing_the_label() {
    let mut app = app();
    app.state.elements = (0..80)
        .map(|i| {
            LabelElement::Object(LabelObject::text(
                format!("PORT{i:03}"),
                TextBox {
                    x_mm: i as f64 * 12.0,
                    ..Default::default()
                },
            ))
        })
        .collect();
    let ctx = egui::Context::default();
    let size = egui::vec2(800.0, 500.0);
    frame(&ctx, &mut app, size, vec![]);
    let original = app.state.document();
    app.state
        .select_canvas_target(Some(crate::state::CanvasTarget {
            element: 79,
            edit: CanvasEditTarget::Content,
        }));
    let mut visible = false;
    for _ in 0..12 {
        let output = frame(&ctx, &mut app, size, vec![]);
        visible |= output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("80. ") => shape
                .clip_rect
                .contains(text.pos + text.galley.rect.center().to_vec2()),
            _ => false,
        });
    }
    assert!(visible);
    assert_eq!(app.state.document(), original);
}

#[test]
fn operating_system_close_request_waits_for_the_unsaved_confirmation() {
    let mut app = app();
    app.state.add_text();
    let ctx = egui::Context::default();
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000.0, 700.0),
        )),
        ..Default::default()
    };
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let mut output = ctx.run_ui(input, |ui| app.show_editor(ui));
    output.textures_delta.clear();
    assert_eq!(app.state.pending_file_action, Some(FileAction::Close));
    assert!(!app.state.close_approved);
    assert!(output.viewport_output.values().any(|viewport| {
        viewport
            .commands
            .contains(&egui::ViewportCommand::CancelClose)
    }));
    file_workflow::apply_choice(&mut app.state, ConfirmChoice::Cancel);
    assert_eq!(app.state.pending_file_action, None);
    assert!(!app.state.close_approved);
}

#[test]
fn resizing_a_list_reveals_its_existing_selection() {
    let mut app = app();
    app.state.elements = (0..30)
        .map(|i| {
            LabelElement::Object(LabelObject::text(
                format!("PORT{i:03}"),
                TextBox {
                    x_mm: i as f64 * 12.0,
                    ..Default::default()
                },
            ))
        })
        .collect();
    let ctx = egui::Context::default();
    app.state
        .select_canvas_target(Some(crate::state::CanvasTarget {
            element: 15,
            edit: CanvasEditTarget::Content,
        }));
    for _ in 0..8 {
        frame(&ctx, &mut app, egui::vec2(1400.0, 900.0), vec![]);
    }
    let mut visible = false;
    for _ in 0..12 {
        let output = frame(&ctx, &mut app, egui::vec2(800.0, 500.0), vec![]);
        visible |= output.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text().starts_with("16. ") => shape
                .clip_rect
                .contains(text.pos + text.galley.rect.center().to_vec2()),
            _ => false,
        });
    }
    assert!(visible);
}

#[test]
fn saving_to_the_same_file_refreshes_the_title_without_a_new_edit_revision() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cached.ptl");
    let mut app = app();
    let ctx = egui::Context::default();
    let size = egui::vec2(1000.0, 700.0);
    app.state.add_text();
    frame(&ctx, &mut app, size, vec![]);
    let revision = app.state.edit_revision;
    assert!(file_workflow::save_to(
        &mut app.state,
        SaveKind::SaveAs,
        Some(&path)
    ));
    let output = frame(&ctx, &mut app, size, vec![]);
    assert_eq!(app.state.edit_revision, revision);
    assert!(output.viewport_output.values().any(|viewport| viewport.commands.iter().any(|command| matches!(command, egui::ViewportCommand::Title(title) if title == "P-Touch Label Printer: cached.ptl"))));
    app.state.add_text();
    frame(&ctx, &mut app, size, vec![]);
    let revision = app.state.edit_revision;
    assert!(file_workflow::save_to(&mut app.state, SaveKind::Save, None));
    let output = frame(&ctx, &mut app, size, vec![]);
    assert_eq!(app.state.edit_revision, revision);
    assert!(output.viewport_output.values().any(|viewport| viewport.commands.iter().any(|command| matches!(command, egui::ViewportCommand::Title(title) if title == "P-Touch Label Printer: cached.ptl"))));
}

#[test]
fn design_dpi_changes_are_saved_changes_with_or_without_a_printer() {
    for connected in [false, true] {
        let ctx = egui::Context::default();
        let size = egui::vec2(1200.0, 800.0);
        let mut app = app();
        app.state.printer_connected = connected;
        app.state.printer_dpi = 180;
        let saved = app.state.document();
        app.state.document_session.reset(&saved);
        let before = app.state.edit_revision;
        let _ = frame(&ctx, &mut app, size, vec![]);
        app.state.set_design_dpi(360);
        assert_eq!(app.state.design_dpi, 360);
        assert_eq!(app.state.target_dpi(), if connected { 180 } else { 360 });
        assert!(app.state.edit_revision > before);
        assert!(
            app.state
                .document_session
                .is_modified(&app.state.document())
        );
        let output = frame(&ctx, &mut app, size, vec![]);
        assert!(output.viewport_output.values().any(|viewport| {
            viewport.commands.iter().any(|command| {
                matches!(command, egui::ViewportCommand::Title(title) if title.contains("unsaved"))
            })
        }));
    }
}

#[test]
fn repeat_modal_blocks_file_and_document_history_shortcuts() {
    let context = egui::Context::default();
    let mut app = app();
    let size = egui::vec2(1400.0, 900.0);
    app.state.add_text();
    frame(&context, &mut app, size, Vec::new());
    app.state.add_text();
    let output = frame(&context, &mut app, size, Vec::new());
    let repeat = text_rect(&output, "Repeat").center();
    frame(&context, &mut app, size, click(repeat));
    assert!(crate::panels::repeat_tools::is_open(&context));
    let before = app.state.document();
    let selection = app.state.selected_indices();
    for command in [egui::Key::Z, egui::Key::Y, egui::Key::N] {
        frame(&context, &mut app, size, vec![key(command, true)]);
        assert_eq!(app.state.document(), before);
        assert_eq!(app.state.selected_indices(), selection);
        assert!(crate::panels::repeat_tools::is_open(&context));
    }
    for command in [egui::Key::O, egui::Key::S] {
        let mut result = None;
        let mut output = context.run_ui(
            egui::RawInput {
                events: vec![key(command, true)],
                ..Default::default()
            },
            |ui| {
                result = file_shortcut(ui.ctx());
            },
        );
        output.textures_delta.clear();
        assert!(result.is_none());
    }
    crate::file_workflow::new_document(&mut app.state);
    frame(&context, &mut app, size, Vec::new());
    assert!(!crate::panels::repeat_tools::is_open(&context));
}
