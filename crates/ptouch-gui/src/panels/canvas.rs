// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Central canvas panel for label preview display.

use crate::state::AppState;

/// Render the central preview canvas.
pub fn show_canvas(ui: &mut egui::Ui, state: &mut AppState) {
    // Zoom controls at the top of the canvas area
    ui.horizontal(|ui| {
        if ui.button("Fit").clicked() {
            state.zoom_fit = true;
        }
        if ui.button("1:1").clicked() {
            state.zoom = 1.0;
            state.zoom_fit = false;
        }
        if ui.button("+").clicked() {
            state.zoom = (state.zoom * 1.25).min(20.0);
            state.zoom_fit = false;
        }
        if ui.button("-").clicked() {
            state.zoom = (state.zoom / 1.25).max(0.1);
            state.zoom_fit = false;
        }
        ui.label(format!("Zoom: {:.0}%", state.zoom * 100.0));
    });

    ui.separator();

    // Canvas area
    let canvas_rect = ui.available_rect_before_wrap();
    let canvas_size = canvas_rect.size();

    // Fill the background with a neutral gray
    ui.painter()
        .rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(200));

    match state.preview_texture {
        Some(ref texture) => {
            let tex_size = texture.size_vec2();

            // Calculate zoom
            let zoom = if state.zoom_fit {
                let zoom_x = (canvas_size.x - 20.0) / tex_size.x;
                let zoom_y = (canvas_size.y - 20.0) / tex_size.y;
                let fit_zoom = zoom_x.min(zoom_y).clamp(0.1, 10.0);
                state.zoom = fit_zoom;
                fit_zoom
            } else {
                state.zoom
            };

            let display_w = tex_size.x * zoom;
            let display_h = tex_size.y * zoom;
            let display_size = egui::vec2(display_w, display_h);

            // Center the label in the canvas
            let center = canvas_rect.center();
            let label_rect = egui::Rect::from_center_size(center, display_size);

            // Draw a white background behind the label (the tape)
            ui.painter()
                .rect_filled(label_rect, 0.0, egui::Color32::WHITE);

            // Draw a thin border around the label
            ui.painter().rect_stroke(
                label_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(128)),
                egui::StrokeKind::Outside,
            );

            // Draw the label texture centered (same rect as background/border)
            ui.painter().image(
                texture.id(),
                label_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );

            // Allocate the space so the panel is not empty
            ui.allocate_rect(canvas_rect, egui::Sense::hover());
        }
        None => {
            // No preview available
            let center = canvas_rect.center();
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                "Add elements to preview",
                egui::FontId::proportional(18.0),
                egui::Color32::from_gray(100),
            );
            // Allocate the space so the panel is not empty
            ui.allocate_rect(canvas_rect, egui::Sense::hover());
        }
    }
}
