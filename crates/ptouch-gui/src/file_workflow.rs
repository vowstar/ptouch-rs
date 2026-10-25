// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Deferred file commands and confirmation for unsaved document changes.

use std::path::Path;

use crate::document_session::{SaveError, SaveKind};
use crate::state::AppState;
use ptouch_render::document::{DEFAULT_DOCUMENT_VERSION, LabelDocument};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAction {
    New,
    Open,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmChoice {
    Save,
    Discard,
    Cancel,
}

pub fn request(state: &mut AppState, action: FileAction) {
    if state.pending_file_action.is_none() {
        state.pending_file_action = Some(action);
    }
}

pub fn save(state: &mut AppState, kind: SaveKind) -> bool {
    let selected = if state.document_session.needs_save_path(kind) {
        let mut dialog = crate::widgets::layout_file_dialog().set_file_name("label.ptl");
        if let Some(path) = state.document_session.current_path() {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = path.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        }
        dialog.save_file()
    } else {
        None
    };
    save_to(state, kind, selected.as_deref())
}

pub fn save_to(state: &mut AppState, kind: SaveKind, selected: Option<&Path>) -> bool {
    let document = state.document();
    match state.document_session.save(kind, selected, &document) {
        Ok(Some(path)) => {
            set_file_status(state, "Saved", &path);
            true
        }
        Ok(None) => false,
        Err(SaveError::Write { path, source }) => {
            let message = format!("Cannot save {}: {source}", file_name(&path));
            state.status_tooltip = Some((message.clone(), path.display().to_string()));
            state.status_message = message;
            false
        }
        Err(error) => {
            state.status_message = error.to_string();
            false
        }
    }
}

pub fn set_file_status(state: &mut AppState, verb: &str, path: &Path) {
    let message = format!("{verb} {}", file_name(path));
    state.status_tooltip = Some((message.clone(), path.display().to_string()));
    state.status_message = message;
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

pub fn open_path(state: &mut AppState, path: &Path) -> Result<(), String> {
    let document = std::fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| LabelDocument::from_toml_str(&text).map_err(|error| error.to_string()))?;
    crate::panels::toolbar::apply_layout(state, document)?;
    state
        .document_session
        .opened(path.to_path_buf(), &state.document());
    state.file_baseline_pending = true;
    state.history = Default::default();
    set_file_status(state, "Opened", path);
    Ok(())
}

pub fn new_document(state: &mut AppState) {
    let document = LabelDocument {
        version: DEFAULT_DOCUMENT_VERSION,
        tape_width_mm: state.tape_width_mm,
        dpi: state.design_dpi,
        font_name: state.font_name.clone(),
        font_margin: state.font_margin,
        flip_h: false,
        flip_v: false,
        elements: Vec::new(),
    };
    state.restore_document(document);
    state.file_view_generation = state.file_view_generation.wrapping_add(1);
    state.select_canvas_target(None);
    state.selection.clear();
    state.document_session.reset(&state.document());
    state.file_baseline_pending = false;
    state.history = Default::default();
    state.status_tooltip = None;
    state.status_message = "New label".into();
}

/// Complete the baseline after the first render resolves automatic geometry.
pub fn finish_open(state: &mut AppState) {
    if state.file_baseline_pending {
        state.file_baseline_pending = false;
        if let Some(path) = state.document_session.current_path().map(Path::to_path_buf) {
            state.document_session.opened(path, &state.document());
        }
    }
}

pub fn apply_choice(state: &mut AppState, choice: ConfirmChoice) -> Option<FileAction> {
    match choice {
        ConfirmChoice::Cancel => {
            state.pending_file_action = None;
            None
        }
        ConfirmChoice::Discard => state.pending_file_action.take(),
        ConfirmChoice::Save => {
            if save(state, SaveKind::Save) {
                state.pending_file_action.take()
            } else {
                None
            }
        }
    }
}

/// Called after the editor commits the current frame's numeric and text edits.
pub fn show_pending(ctx: &egui::Context, state: &mut AppState) {
    let Some(action) = state.pending_file_action else {
        return;
    };
    if state.canvas_drag_target.is_some() {
        return;
    }
    let action = if is_modified(ctx, state) {
        let response = egui::Modal::new(egui::Id::new("unsaved_changes")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(match action {
                FileAction::New => "Save changes before you make a new label?",
                FileAction::Open => "Save changes before you open another file?",
                FileAction::Close => "Save changes before you close the program?",
            });
            if state.status_message.starts_with("Cannot save") {
                ui.colored_label(ui.visuals().error_fg_color, &state.status_message);
            }
            ui.horizontal(|ui| {
                for (label, choice) in [
                    ("Save", ConfirmChoice::Save),
                    ("Discard", ConfirmChoice::Discard),
                    ("Cancel", ConfirmChoice::Cancel),
                ] {
                    if ui.button(label).clicked() {
                        return Some(choice);
                    }
                }
                None
            })
            .inner
        });
        if let Some(choice) = response.inner {
            apply_choice(state, choice)
        } else if response.should_close() {
            apply_choice(state, ConfirmChoice::Cancel)
        } else {
            None
        }
    } else {
        state.pending_file_action.take()
    };
    match action {
        Some(FileAction::New) => new_document(state),
        Some(FileAction::Open) => {
            let mut dialog = crate::widgets::layout_file_dialog();
            if let Some(parent) = state.document_session.current_path().and_then(Path::parent) {
                dialog = dialog.set_directory(parent);
            }
            if let Some(path) = dialog.pick_file()
                && let Err(error) = open_path(state, &path)
            {
                let message = format!("Cannot open {}: {error}", file_name(&path));
                state.status_tooltip = Some((message.clone(), path.display().to_string()));
                state.status_message = message;
            }
        }
        Some(FileAction::Close) => {
            state.close_approved = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        None => {}
    }
}

#[cfg(test)]
pub fn window_title(state: &AppState) -> String {
    title_with_modified(state, state.document_session.is_modified(&state.document()))
}

fn title_with_modified(state: &AppState, is_modified: bool) -> String {
    let name = state
        .document_session
        .current_path()
        .map(file_name)
        .unwrap_or_else(|| "Untitled".into());
    let modified = if is_modified { " (unsaved)" } else { "" };
    format!("P-Touch Label Printer: {name}{modified}")
}

pub fn update_title(ctx: &egui::Context, state: &AppState) {
    let title = title_with_modified(state, is_modified(ctx, state));
    let changed = ctx.data_mut(|data| {
        let key = egui::Id::new("last_window_title");
        if data.get_temp::<String>(key).as_deref() == Some(&title) {
            false
        } else {
            data.insert_temp(key, title.clone());
            true
        }
    });
    if changed {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

fn is_modified(ctx: &egui::Context, state: &AppState) -> bool {
    let key = egui::Id::new("document_modified");
    let revision = (state.edit_revision, state.document_session.generation());
    let cached = ctx.data_mut(|data| data.get_temp::<(u64, u64, bool)>(key));
    if let Some((edit, saved, modified)) = cached
        && (edit, saved) == revision
    {
        return modified;
    }
    let modified = state.document_session.is_modified(&state.document());
    ctx.data_mut(|data| data.insert_temp(key, (revision.0, revision.1, modified)));
    modified
}
