// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Right properties panel for editing selected element attributes.

use log::{error, info};
use std::path::PathBuf;

use ptouch_render::image_loader;
use ptouch_render::text::TextAlign;

use crate::state::{AppState, LabelElement};

/// Render the right-side properties panel.
pub fn show_properties(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Properties");
    ui.add_space(4.0);

    let selected = match state.selected_element {
        Some(idx) if idx < state.elements.len() => idx,
        _ => {
            ui.label("Select an element to edit properties");
            return;
        }
    };

    // We need to clone the element to avoid borrow issues, then write back.
    let mut element = state.elements[selected].clone();
    let mut changed = false;

    match &mut element {
        LabelElement::Text {
            content,
            font_size,
            align,
            rotation,
        } => {
            changed |= show_text_properties(ui, content, font_size, align, rotation, state);
        }
        LabelElement::Image { path, bitmap } => {
            changed |= show_image_properties(ui, path, bitmap);
        }
        LabelElement::CutMark => {
            ui.label("Cut Mark");
            ui.add_space(4.0);
            ui.label("No editable properties.");
        }
        LabelElement::Padding { pixels } => {
            changed |= show_padding_properties(ui, pixels);
        }
    }

    if changed {
        state.elements[selected] = element;
        state.mark_dirty();
    }
}

/// Show properties for a text element. Returns true if any value changed.
fn show_text_properties(
    ui: &mut egui::Ui,
    content: &mut String,
    font_size: &mut Option<f32>,
    align: &mut TextAlign,
    rotation: &mut f32,
    state: &mut AppState,
) -> bool {
    let mut changed = false;

    ui.label("Text Content:");
    let response = ui.add(
        egui::TextEdit::multiline(content)
            .desired_width(f32::INFINITY)
            .desired_rows(4),
    );
    if response.changed() {
        changed = true;
    }

    ui.add_space(8.0);

    // Font name (dropdown ComboBox with search)
    ui.label("Font:");
    egui::ComboBox::from_id_salt("font_selector")
        .selected_text(&state.font_name)
        .width(150.0)
        .height(300.0)
        .show_ui(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.font_search)
                    .desired_width(f32::INFINITY)
                    .hint_text("Search..."),
            );
            ui.add_space(2.0);
            let query = state.font_search.to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(250.0)
                .show(ui, |ui| {
                    for font in &state.available_fonts {
                        if !query.is_empty() && !font.to_lowercase().contains(&query) {
                            continue;
                        }
                        if ui
                            .selectable_label(state.font_name == *font, font)
                            .clicked()
                        {
                            state.font_name = font.clone();
                            state.font_search.clear();
                            changed = true;
                        }
                    }
                });
        });

    ui.add_space(4.0);

    // Font size
    let mut use_auto = font_size.is_none();
    if ui.checkbox(&mut use_auto, "Auto font size").changed() {
        if use_auto {
            *font_size = None;
        } else {
            *font_size = Some(24.0);
        }
        changed = true;
    }

    if let Some(ref mut size) = font_size {
        ui.horizontal(|ui| {
            ui.label("Size:");
            if ui
                .add(egui::DragValue::new(size).speed(0.5).range(4.0..=200.0))
                .changed()
            {
                changed = true;
            }
        });
    }

    ui.add_space(4.0);

    // Font margin
    ui.horizontal(|ui| {
        ui.label("Margin:");
        if ui
            .add(
                egui::DragValue::new(&mut state.font_margin)
                    .speed(1.0)
                    .range(0..=50),
            )
            .changed()
        {
            changed = true;
        }
        ui.label("px");
    });

    ui.add_space(4.0);

    // Alignment
    ui.label("Alignment:");
    ui.horizontal(|ui| {
        if ui
            .selectable_label(matches!(align, TextAlign::Left), "Left")
            .clicked()
        {
            *align = TextAlign::Left;
            changed = true;
        }
        if ui
            .selectable_label(matches!(align, TextAlign::Center), "Center")
            .clicked()
        {
            *align = TextAlign::Center;
            changed = true;
        }
        if ui
            .selectable_label(matches!(align, TextAlign::Right), "Right")
            .clicked()
        {
            *align = TextAlign::Right;
            changed = true;
        }
    });

    ui.add_space(4.0);

    // Rotation
    ui.label("Rotation:");
    ui.horizontal(|ui| {
        if ui
            .add(
                egui::DragValue::new(rotation)
                    .speed(1.0)
                    .range(-360.0..=360.0)
                    .suffix(" deg"),
            )
            .changed()
        {
            state.rotation_input = format!("{}", *rotation as i32);
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        for &deg in &[0.0_f32, 45.0, 90.0, 135.0, 180.0, 270.0] {
            if ui
                .selectable_label((*rotation - deg).abs() < 0.5, format!("{}", deg as i32))
                .clicked()
            {
                *rotation = deg;
                state.rotation_input = format!("{}", deg as i32);
                changed = true;
            }
        }
    });

    // Manual angle input field
    ui.horizontal(|ui| {
        ui.label("Angle:");
        let resp = ui.add(
            egui::TextEdit::singleline(&mut state.rotation_input)
                .desired_width(60.0)
                .hint_text("deg"),
        );
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Ok(val) = state.rotation_input.parse::<f32>() {
                let clamped = val.clamp(-360.0, 360.0);
                *rotation = clamped;
                state.rotation_input = format!("{}", clamped);
                changed = true;
            }
        }
    });

    changed
}

/// Show properties for an image element. Returns true if changed.
fn show_image_properties(
    ui: &mut egui::Ui,
    path: &mut PathBuf,
    bitmap: &mut Option<ptouch_render::bitmap::LabelBitmap>,
) -> bool {
    let mut changed = false;

    ui.label("Image");
    ui.add_space(4.0);

    ui.label(format!("File: {}", path.display()));

    if let Some(ref bmp) = bitmap {
        ui.label(format!("Size: {} x {} px", bmp.width(), bmp.height()));
    }

    ui.add_space(4.0);

    if ui.button("Reload").clicked() {
        match image_loader::load_png(path) {
            Ok(bmp) => {
                info!("Reloaded image: {}", path.display());
                *bitmap = Some(bmp);
                changed = true;
            }
            Err(e) => {
                error!("Reload failed: {}", e);
            }
        }
    }

    if ui.button("Change File...").clicked() {
        if let Some(new_path) = rfd::FileDialog::new()
            .add_filter("PNG Images", &["png"])
            .pick_file()
        {
            match image_loader::load_png(&new_path) {
                Ok(bmp) => {
                    info!("Changed image to: {}", new_path.display());
                    *path = new_path;
                    *bitmap = Some(bmp);
                    changed = true;
                }
                Err(e) => {
                    error!("Load failed: {}", e);
                }
            }
        }
    }

    changed
}

/// Show properties for a padding element. Returns true if changed.
fn show_padding_properties(ui: &mut egui::Ui, pixels: &mut u32) -> bool {
    let mut changed = false;

    ui.label("Padding");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.label("Pixels:");
        if ui
            .add(egui::DragValue::new(pixels).speed(1.0).range(1..=500))
            .changed()
        {
            changed = true;
        }
    });

    ui.add_space(4.0);

    // Quick preset buttons
    ui.horizontal(|ui| {
        for &preset in &[5, 10, 20, 50, 100] {
            if ui.button(format!("{}", preset)).clicked() {
                *pixels = preset;
                changed = true;
            }
        }
    });

    changed
}
