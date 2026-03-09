// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Application state for the P-Touch GUI.

use std::path::PathBuf;

use ptouch_render::bitmap::LabelBitmap;
use ptouch_render::text::TextAlign;

/// A single element in the label composition.
#[derive(Debug, Clone)]
pub enum LabelElement {
    /// A text block with content, optional font size, alignment, and rotation.
    Text {
        content: String,
        font_size: Option<f32>,
        align: TextAlign,
        /// Rotation angle in degrees (clockwise). 0.0 = horizontal.
        rotation: f32,
    },
    /// An image loaded from a file.
    Image {
        path: PathBuf,
        bitmap: Option<LabelBitmap>,
    },
    /// A cut mark separator.
    CutMark,
    /// Horizontal padding in pixels.
    Padding { pixels: u32 },
}

impl LabelElement {
    /// Returns a short display name for the element list.
    pub fn display_name(&self) -> String {
        match self {
            LabelElement::Text { content, .. } => {
                let preview: String = content.chars().take(20).collect();
                if content.len() > 20 {
                    format!("Text: {}...", preview)
                } else {
                    format!("Text: {}", preview)
                }
            }
            LabelElement::Image { path, .. } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                format!("Image: {}", name)
            }
            LabelElement::CutMark => "Cut Mark".to_string(),
            LabelElement::Padding { pixels } => format!("Padding: {} px", pixels),
        }
    }
}

/// Central application state shared across all panels.
pub struct AppState {
    /// List of label elements in composition order.
    pub elements: Vec<LabelElement>,
    /// Index of the currently selected element, if any.
    pub selected_element: Option<usize>,
    /// Current tape width in millimeters.
    pub tape_width_mm: u8,
    /// Current tape width in pixels (derived from tape_width_mm).
    pub tape_width_px: u32,
    /// Font name used for text rendering.
    pub font_name: String,
    /// Font top/bottom margin in pixels.
    pub font_margin: u32,
    /// Cached list of available system font family names.
    pub available_fonts: Vec<String>,
    /// The rendered preview bitmap (1-bit).
    pub preview_bitmap: Option<LabelBitmap>,
    /// The preview texture uploaded to the GPU.
    pub preview_texture: Option<egui::TextureHandle>,
    /// Flag indicating the preview needs to be re-rendered.
    pub needs_rerender: bool,
    /// Current zoom level (1.0 = 100%).
    pub zoom: f32,
    /// Whether zoom should auto-fit to the canvas.
    pub zoom_fit: bool,
    /// Printer connection status message.
    pub printer_status: Option<String>,
    /// Detected printer model name.
    pub printer_model: Option<String>,
    /// Status bar message for transient feedback.
    pub status_message: String,
    /// Buffer for manual rotation angle input in properties panel.
    pub rotation_input: String,
    /// Buffer for font search/filter in properties panel.
    pub font_search: String,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            elements: Vec::new(),
            selected_element: None,
            tape_width_mm: 12,
            tape_width_px: 76,
            font_name: "DejaVuSans".to_string(),
            font_margin: 0,
            available_fonts: Vec::new(),
            preview_bitmap: None,
            preview_texture: None,
            needs_rerender: true,
            zoom: 1.0,
            zoom_fit: true,
            printer_status: None,
            printer_model: None,
            status_message: "Ready".to_string(),
            rotation_input: String::new(),
            font_search: String::new(),
        }
    }
}

impl AppState {
    /// Update the tape width in pixels based on the current tape_width_mm.
    pub fn update_tape_pixels(&mut self) {
        if let Some(tape) = ptouch_core::tape::find_tape(self.tape_width_mm) {
            self.tape_width_px = u32::from(tape.pixels);
        }
    }

    /// Mark the preview as needing re-render.
    pub fn mark_dirty(&mut self) {
        self.needs_rerender = true;
    }

    /// Ensure the selected element index is valid.
    pub fn validate_selection(&mut self) {
        if let Some(idx) = self.selected_element {
            if idx >= self.elements.len() {
                self.selected_element = if self.elements.is_empty() {
                    None
                } else {
                    Some(self.elements.len() - 1)
                };
            }
        }
    }
}
