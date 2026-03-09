// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Main application struct and eframe::App implementation.

use log::{error, info};

use ptouch_render::bitmap::LabelBitmap;
use ptouch_render::compose;
use ptouch_render::image_loader;
use ptouch_render::text::TextRenderer;

use crate::panels;
use crate::state::{AppState, LabelElement};

/// The main P-Touch GUI application.
pub struct PtouchApp {
    /// Application state shared across all panels.
    pub state: AppState,
    /// Text renderer instance for generating label bitmaps.
    renderer: TextRenderer,
}

impl PtouchApp {
    /// Create a new application instance.
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            state: AppState {
                available_fonts: ptouch_render::font::list_fonts(),
                ..AppState::default()
            },
            renderer: TextRenderer::new(),
        }
    }

    /// Re-render the preview bitmap from the current element list.
    pub fn update_preview(&mut self, ctx: &egui::Context) {
        self.state.needs_rerender = false;

        if self.state.elements.is_empty() {
            self.state.preview_bitmap = None;
            self.state.preview_texture = None;
            return;
        }

        let print_width = self.state.tape_width_px;
        let mut result: Option<LabelBitmap> = None;

        for element in &self.state.elements {
            let segment = match element {
                LabelElement::Text {
                    content,
                    font_size,
                    align,
                    rotation,
                } => {
                    if content.is_empty() {
                        continue;
                    }
                    let lines: Vec<&str> = content.lines().collect();

                    let norm = ((*rotation % 360.0) + 360.0) % 360.0;
                    let is_rotated = !(norm.abs() < 0.5 || (norm - 360.0).abs() < 0.5);

                    // For rotated text with auto font size, calculate a size
                    // that fits within tape_height after rotation.
                    let effective_font_size =
                        rotation_aware_font_size(*font_size, *rotation, &lines, print_width);

                    // For rotated text, use a taller render area so all lines
                    // are visible (the height becomes tape length after rotation).
                    let render_height = if is_rotated {
                        if let Some(fs) = effective_font_size {
                            let line_h = (fs * 1.2).ceil();
                            let text_h = (lines.len() as f32 * line_h).ceil() as u32
                                + self.state.font_margin * 2;
                            text_h.max(print_width)
                        } else {
                            print_width
                        }
                    } else {
                        print_width
                    };

                    let bmp = match self.renderer.render_text(
                        &lines,
                        render_height,
                        &self.state.font_name,
                        effective_font_size,
                        self.state.font_margin,
                        *align,
                    ) {
                        Ok(bmp) => bmp,
                        Err(e) => {
                            error!("Text render failed: {}", e);
                            self.state.status_message = format!("Text render error: {}", e);
                            continue;
                        }
                    };

                    if is_rotated {
                        // Trim whitespace so the rotated bounding box reflects
                        // actual text content, not full tape-height padding.
                        bmp.trim_vertical()
                            .rotate(*rotation)
                            .fit_height(print_width)
                    } else {
                        bmp
                    }
                }
                LabelElement::Image { path, bitmap } => {
                    if let Some(bmp) = bitmap {
                        bmp.clone()
                    } else {
                        match image_loader::load_image(
                            path,
                            &image_loader::ImageLoadOptions::default(),
                        ) {
                            Ok(bmp) => bmp,
                            Err(e) => {
                                error!("Image load failed: {}", e);
                                self.state.status_message = format!("Image load error: {}", e);
                                continue;
                            }
                        }
                    }
                }
                LabelElement::CutMark => compose::cutmark(print_width),
                LabelElement::Padding { pixels } => compose::padding(print_width, *pixels),
            };

            result = Some(match result {
                Some(prev) => prev.append(&segment),
                None => segment,
            });
        }

        if let Some(ref bitmap) = result {
            let rgba = bitmap.to_rgba_image();
            let max_side = ctx.input(|i| i.max_texture_side);

            let rgba = if rgba.width() as usize > max_side || rgba.height() as usize > max_side {
                let scale = max_side as f32 / rgba.width().max(rgba.height()) as f32;
                let new_w = (rgba.width() as f32 * scale).floor() as u32;
                let new_h = (rgba.height() as f32 * scale).floor() as u32;
                image::imageops::resize(
                    &rgba,
                    new_w.max(1),
                    new_h.max(1),
                    image::imageops::FilterType::Nearest,
                )
            } else {
                rgba
            };

            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);

            let texture =
                ctx.load_texture("label_preview", color_image, egui::TextureOptions::NEAREST);
            self.state.preview_texture = Some(texture);
        } else {
            self.state.preview_texture = None;
        }

        self.state.preview_bitmap = result;
        info!("Preview updated");
    }
}

/// Calculate font size that fits within `tape_height` after rotation.
///
/// For 0 degrees, returns the original font_size (None = let renderer auto-size).
/// For other angles, estimates the maximum font size whose rotated bounding box
/// fits within the tape height.
fn rotation_aware_font_size(
    font_size: Option<f32>,
    rotation_deg: f32,
    lines: &[&str],
    tape_height: u32,
) -> Option<f32> {
    // User-specified font size: use it directly, no auto-adjustment
    if font_size.is_some() {
        return font_size;
    }

    let norm = ((rotation_deg % 360.0) + 360.0) % 360.0;
    // No rotation or effectively 0/360: let renderer auto-size normally
    if norm.abs() < 0.5 || (norm - 360.0).abs() < 0.5 {
        return None;
    }

    let angle_rad = norm.to_radians();
    let sin_a = angle_rad.sin().abs();
    let cos_a = angle_rad.cos().abs();
    let num_lines = lines.len().max(1) as f32;
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
    let available = tape_height as f32;

    // Rotated bounding box height:
    //   bbox_h = text_width * |sin| + text_height * |cos|
    // where text_width ~ max_chars * font_size * 0.6
    //       text_height ~ num_lines * font_size * 1.2
    // Solve for font_size:
    //   font_size = available / (max_chars * 0.6 * |sin| + num_lines * 1.2 * |cos|)
    let denom = max_chars * 0.6 * sin_a + num_lines * 1.2 * cos_a;
    if denom > 0.01 {
        Some((available / denom).max(4.0))
    } else {
        None
    }
}

impl eframe::App for PtouchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top toolbar
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            panels::toolbar::show_toolbar(ui, &mut self.state);
        });

        // Bottom status bar
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            panels::status_bar::show_status_bar(ui, &self.state);
        });

        // Left sidebar
        egui::SidePanel::left("sidebar")
            .default_width(200.0)
            .resizable(true)
            .show(ctx, |ui| {
                panels::sidebar::show_sidebar(ui, &mut self.state);
            });

        // Right properties panel
        egui::SidePanel::right("properties")
            .default_width(250.0)
            .resizable(true)
            .show(ctx, |ui| {
                panels::properties::show_properties(ui, &mut self.state);
            });

        // Central canvas
        egui::CentralPanel::default().show(ctx, |ui| {
            panels::canvas::show_canvas(ui, &mut self.state);
        });

        // Re-render preview if dirty
        if self.state.needs_rerender {
            self.update_preview(ctx);
        }
    }
}
