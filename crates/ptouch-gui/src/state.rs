// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Application state for the P-Touch GUI.

use std::sync::mpsc;

use ptouch_core::protocol::PrintQuality;
use ptouch_render::bitmap::LabelBitmap;
use ptouch_render::document::{
    ContentBox, ContentGeometry, DEFAULT_DOCUMENT_VERSION, ElementBounds, LabelDocument,
    LabelObject, ObjectContent, PositionMode, TextBox,
};
use ptouch_render::text::{TextAlign, TextRenderer};

pub use ptouch_render::document::LabelElement;

/// Operation on the selected object. Layout is entered explicitly in properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanvasEditTarget {
    #[default]
    Content,
    Slot,
}

/// A specific object or template slot addressed by the canvas and element list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasTarget {
    pub element: usize,
    pub edit: CanvasEditTarget,
}

/// Printer connection selected in the GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterTarget {
    Usb,
    UsbAt(ptouch_core::transport::UsbLocation),
    #[cfg(windows)]
    UsbPrint {
        instance_id: String,
    },
    #[cfg(any(target_os = "macos", test))]
    Bluetooth {
        name: String,
        address: String,
    },
}

impl PrinterTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Usb => "USB (automatic)".to_string(),
            Self::UsbAt(location) => format!("USB {location}"),
            #[cfg(windows)]
            Self::UsbPrint { instance_id } => format!("PT-P710BT ({instance_id})"),
            #[cfg(any(target_os = "macos", test))]
            Self::Bluetooth { name, address } => format!("{name} ({address})"),
        }
    }

    pub fn usb_location(&self) -> Option<ptouch_core::transport::UsbLocation> {
        match self {
            Self::UsbAt(location) => Some(*location),
            _ => None,
        }
    }

    pub fn is_bluetooth(&self) -> bool {
        match self {
            Self::Usb | Self::UsbAt(_) => false,
            #[cfg(windows)]
            Self::UsbPrint { .. } => false,
            #[cfg(any(target_os = "macos", test))]
            Self::Bluetooth { .. } => true,
        }
    }
}

/// Commands sent from the UI thread to the printer worker.
pub enum PrinterCommand {
    /// Find paired Bluetooth printers supported by this application.
    DiscoverBluetooth,
    /// Poll for a connected printer (query status only, no init).
    Poll(PrinterTarget),
    /// Print raster data.
    Print {
        raster_lines: Vec<Vec<u8>>,
        chain_print: bool,
        auto_cut: bool,
        quality: PrintQuality,
        target: PrinterTarget,
        cancellation: ptouch_core::CancellationToken,
    },
    /// Feed tape forward and cut.
    FeedAndCut {
        target: PrinterTarget,
        cancellation: ptouch_core::CancellationToken,
    },
}

/// A worker response and the printer it belongs to. Discovery is global.
pub struct PrinterEvent {
    pub target: Option<PrinterTarget>,
    pub response: PrinterResponse,
}

/// Responses sent from the printer worker back to the UI thread.
pub enum PrinterResponse {
    /// Paired PT-P300BT printers found by macOS.
    BluetoothDevices(Vec<PrinterTarget>),
    /// Detected USB locations for explicit selection.
    UsbDevices(Vec<PrinterTarget>),
    /// A printer was found and its status queried.
    Connected {
        model_name: String,
        media_width: u8,
        media_type: String,
        max_px: u16,
        dpi: u16,
        quality_modes: bool,
        tape_width_px: u16,
    },
    /// No printer found or previously connected printer lost.
    ConnectionError(String),
    /// Print job completed successfully.
    PrintDone,
    /// Feed and cut completed successfully.
    FeedAndCutDone,
    /// An operation failed.
    Error(String),
}

/// Central application state shared across all panels.
pub struct AppState {
    pub history: crate::history::DocumentHistory,
    pub edit_revision: u64,
    /// Preserve an opened legacy format until the user explicitly converts it.
    pub document_version: u32,
    /// List of label elements in composition order.
    pub elements: Vec<LabelElement>,
    /// Index of the currently selected element, if any.
    pub selected_element: Option<usize>,
    /// Isolate unfinished widget drafts across selections and structural edits.
    pub selection_revision: u64,
    /// Invalidate per-index rich editor histories on structural changes.
    pub editor_generation: u64,
    /// Current tape width in millimeters.
    pub tape_width_mm: u8,
    /// Current tape width in pixels (derived from tape_width_mm).
    pub tape_width_px: u32,
    /// Saved design resolution, used when working offline.
    pub design_dpi: u16,
    /// Keep the last printer geometry through a transient disconnect.
    pub preview_uses_printer: bool,
    /// Font name used for text rendering.
    pub font_name: String,
    /// Font top/bottom margin in pixels.
    pub font_margin: u32,
    /// Mirror the whole composed label left-right (horizontal).
    pub overall_flip_h: bool,
    /// Mirror the whole composed label top-bottom (vertical).
    pub overall_flip_v: bool,
    /// Cached list of available system font family names.
    pub available_fonts: Vec<String>,
    /// The rendered preview bitmap (1-bit).
    pub preview_bitmap: Option<LabelBitmap>,
    /// The preview texture uploaded to the GPU.
    pub preview_texture: Option<egui::TextureHandle>,
    /// Bounds in original print pixels, independent of preview downsampling.
    pub preview_bounds: Vec<ElementBounds>,
    /// Oriented object geometry in original print pixels, after all mirrors.
    pub preview_content_bounds: Vec<ContentGeometry>,
    pub canvas_edit_target: CanvasEditTarget,
    pub canvas_hover_target: Option<CanvasTarget>,
    pub canvas_drag_target: Option<CanvasTarget>,
    pub preview_diagnostics: Vec<String>,
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
    /// Calibration is opt-in for each session and never part of a layout.
    pub calibration_enabled: bool,
    pub calibration: ptouch_render::calibration::PrintCalibration,
    pub calibration_profiles:
        std::collections::BTreeMap<String, ptouch_render::calibration::PrintCalibration>,
    /// Auto-cut after printing. When false, chain print mode (no cut).
    pub auto_cut: bool,
    /// Whether a printer is currently connected (detected by background poll).
    pub printer_connected: bool,
    /// Target selected for polling and printing.
    pub printer_target: PrinterTarget,
    /// Paired PT-P300BT targets discovered on macOS.
    pub bluetooth_targets: Vec<PrinterTarget>,
    /// Whether a printer operation (print, feed & cut) is in progress.
    pub operation_in_progress: bool,
    pub cancellation: Option<ptouch_core::CancellationToken>,
    pub usb_targets: Vec<PrinterTarget>,
    /// Whether the selected printer's status is being requested.
    pub connecting: bool,
    /// Maximum printable pixels of the last connected printer (0 initially).
    pub printer_max_px: u16,
    /// Print resolution of the last connected printer (180 initially).
    /// Kept across disconnects so the canvas does not resize on a
    /// transient USB glitch.
    pub printer_dpi: u16,
    /// Whether the last connected printer supports print quality modes.
    pub printer_quality_modes: bool,
    /// Selected print quality for the next print job.
    pub print_quality: PrintQuality,
    /// Channel sender for commands to the printer worker thread.
    pub printer_cmd_tx: Option<mpsc::Sender<PrinterCommand>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            history: Default::default(),
            edit_revision: 0,
            document_version: DEFAULT_DOCUMENT_VERSION,
            elements: Vec::new(),
            selected_element: None,
            selection_revision: 0,
            editor_generation: 0,
            tape_width_mm: 12,
            tape_width_px: 76,
            design_dpi: 180,
            preview_uses_printer: false,
            font_name: "DejaVuSans".to_string(),
            font_margin: 0,
            overall_flip_h: false,
            overall_flip_v: false,
            available_fonts: Vec::new(),
            preview_bitmap: None,
            preview_texture: None,
            preview_bounds: Vec::new(),
            preview_content_bounds: Vec::new(),
            canvas_edit_target: CanvasEditTarget::default(),
            canvas_hover_target: None,
            canvas_drag_target: None,
            preview_diagnostics: Vec::new(),
            needs_rerender: true,
            zoom: 1.0,
            zoom_fit: true,
            printer_status: None,
            printer_model: None,
            status_message: "Ready".to_string(),
            rotation_input: String::new(),
            font_search: String::new(),
            calibration_enabled: false,
            calibration: ptouch_render::calibration::PrintCalibration {
                name: "Custom printer".into(),
                offset_mm: 0.0,
            },
            calibration_profiles: Default::default(),
            auto_cut: true,
            printer_connected: false,
            printer_target: PrinterTarget::Usb,
            bluetooth_targets: Vec::new(),
            operation_in_progress: false,
            cancellation: None,
            usb_targets: Vec::new(),
            connecting: false,
            printer_max_px: 0,
            printer_dpi: 180,
            printer_quality_modes: false,
            print_quality: PrintQuality::Standard,
            printer_cmd_tx: None,
        }
    }
}

impl AppState {
    pub fn use_flow_allocation(object: &mut LabelObject, height: f64) {
        use ptouch_render::object_layout::{Placement, WidthSizing};
        if object
            .layout
            .as_ref()
            .is_none_or(|p| p.placement != Placement::Free)
        {
            return;
        }
        if let Some(content) = &mut object.content_box {
            let sign = if object.flip_v { -1.0 } else { 1.0 };
            let center =
                object.frame.y_mm + object.frame.height_mm / 2.0 + content.offset_y_mm * sign;
            object.frame.y_mm = 0.0;
            object.frame.height_mm = height;
            content.offset_y_mm = (center - height / 2.0) * sign;
        }
        let policy = object.layout.as_mut().unwrap();
        policy.placement = Placement::Flow;
        policy.width = WidthSizing::Fixed;
    }
    /// A flow chain starts at an absolute anchor and ends before the next one.
    pub fn flow_chain(&self, index: usize) -> Option<std::ops::RangeInclusive<usize>> {
        self.elements.get(index)?.as_object()?;
        let mut start = index;
        while start > 0
            && slot_frame(&self.elements[start])?.position == PositionMode::Relative
            && self.elements[start - 1].as_object().is_some()
        {
            start -= 1;
        }
        let end = self
            .slot_followers(start)
            .into_iter()
            .take_while(|i| self.elements[*i].as_object().is_some())
            .last()
            .unwrap_or(start);
        let object = self.elements[start].as_object()?;
        (end > start
            || object.frame.position == PositionMode::Relative
            || object
                .layout
                .as_ref()
                .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Flow))
        .then_some(start..=end)
    }

    pub fn can_swap_positions(&self, a: usize, b: usize) -> bool {
        if a == b || a >= self.elements.len() || b >= self.elements.len() {
            return false;
        }
        let free = |i| {
            self.flow_chain(i).is_none()
                && self.elements[i].as_object().is_some_and(|o| {
                    o.frame.position == PositionMode::Absolute
                        && o.layout.as_ref().is_none_or(|p| {
                            p.placement == ptouch_render::object_layout::Placement::Free
                        })
                })
        };
        free(a) && free(b)
    }

    pub fn swap_positions(&mut self, a: usize, b: usize) -> bool {
        if !self.can_swap_positions(a, b) {
            self.status_message = "Use sequence reordering for these objects".into();
            return false;
        }
        let first = self.elements[a].as_object().unwrap().frame.clone();
        let second = self.elements[b].as_object().unwrap().frame.clone();
        if first.y_mm + second.height_mm > self.printable_height_mm()
            || second.y_mm + first.height_mm > self.printable_height_mm()
        {
            self.status_message = "Swapped positions would exceed the printable height".into();
            return false;
        }
        for (index, x, y) in [(a, second.x_mm, second.y_mm), (b, first.x_mm, first.y_mm)] {
            let frame = &mut self.elements[index].as_object_mut().unwrap().frame;
            frame.x_mm = x;
            frame.y_mm = y;
        }
        self.validate_selection();
        self.mark_dirty();
        true
    }

    pub fn can_swap_content(&self, a: usize, b: usize) -> bool {
        a != b
            && [a, b].into_iter().all(|i| {
                self.elements
                    .get(i)
                    .and_then(|e| e.as_object())
                    .is_some_and(|o| {
                        o.layout.as_ref().is_none_or(|p| {
                            p.placement == ptouch_render::object_layout::Placement::Flow
                                && p.width == ptouch_render::object_layout::WidthSizing::Fixed
                        })
                    })
            })
    }

    pub fn swap_content(&mut self, a: usize, b: usize) -> bool {
        if !self.can_swap_content(a, b) {
            self.status_message = "Use fixed allocations before swapping template content".into();
            return false;
        }
        if a == b {
            return false;
        }
        let (Some(mut first), Some(mut second)) = (
            self.elements.get(a).and_then(|e| e.as_object()).cloned(),
            self.elements.get(b).and_then(|e| e.as_object()).cloned(),
        ) else {
            return false;
        };
        let before_a = first.clone();
        let before_b = second.clone();
        std::mem::swap(&mut first.content, &mut second.content);
        std::mem::swap(&mut first.content_box, &mut second.content_box);
        std::mem::swap(&mut first.rotation, &mut second.rotation);
        std::mem::swap(&mut first.flip_h, &mut second.flip_h);
        std::mem::swap(&mut first.flip_v, &mut second.flip_v);
        std::mem::swap(&mut first.border, &mut second.border);
        if let Err(message) = self
            .constrain_transform(a, &before_a, &mut first)
            .and_then(|_| self.constrain_transform(b, &before_b, &mut second))
        {
            self.status_message = message.into();
            return false;
        }
        self.elements[a] = LabelElement::Object(first);
        self.elements[b] = LabelElement::Object(second);
        self.validate_selection();
        self.mark_dirty();
        true
    }

    /// Keep the chain anchor at its physical position while content order changes.
    /// Free objects retain coordinates; crossing an anchor never changes bindings.
    pub fn reorder_object(&mut self, from: usize, to: usize) -> bool {
        if from == to || from >= self.elements.len() || to >= self.elements.len() {
            return false;
        }
        let chain = self.flow_chain(from);
        if chain.as_ref().is_some_and(|r| !r.contains(&to))
            || chain.is_none() && self.flow_chain(to).is_some()
        {
            self.status_message =
                "Reorder within this sequence; change positioning to move between sequences".into();
            return false;
        }
        let anchor = chain.as_ref().map(|r| {
            let start = *r.start();
            let f = self.elements[start].as_object().unwrap().frame.clone();
            let gap = self
                .elements
                .get(start + 1)
                .and_then(|e| e.as_object())
                .map_or(3.0, |o| o.frame.gap_mm);
            (start, *r.end(), f.x_mm, gap, f.position, f.gap_mm)
        });
        let element = self.elements.remove(from);
        self.elements.insert(to, element);
        if let Some((start, end, x, gap, position, first_gap)) = anchor {
            for i in start..=end {
                let object = self.elements[i].as_object_mut().unwrap();
                if i == start {
                    object.frame.position = position;
                    object.frame.x_mm = x;
                    object.frame.gap_mm = first_gap;
                } else {
                    if object.frame.position == PositionMode::Absolute {
                        object.frame.gap_mm = gap;
                    }
                    object.frame.position = PositionMode::Relative;
                    object.frame.x_mm = 0.0;
                }
            }
        }
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.select_canvas_target(Some(CanvasTarget {
            element: to,
            edit: self.canvas_edit_target,
        }));
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.mark_dirty();
        true
    }

    /// Normalize free content into its own bounding frame, or clamp offsets
    /// inside a declared allocation. Rotation/size that cannot fit is rejected.
    pub fn constrain_transform(
        &self,
        index: usize,
        before: &LabelObject,
        candidate: &mut LabelObject,
    ) -> Result<bool, &'static str> {
        use ptouch_render::object_layout::{Placement, WidthSizing, content_extent};
        if let (Some(border), Some(old), Some(new)) = (
            &candidate.border,
            &before.content_box,
            &candidate.content_box,
        ) && (old.width_mm != new.width_mm || old.height_mm != new.height_mm)
        {
            let inset = border
                .inset_mm(self.target_dpi())
                .map_err(|_| "Invalid border dimensions")?;
            if new.width_mm <= 2.0 * inset || new.height_mm <= 2.0 * inset {
                return Err(
                    "Border leaves no room for content — enlarge the object or reduce border spacing",
                );
            }
        }
        let Some(policy) = candidate.layout.clone() else {
            return Ok(false);
        };
        if policy.placement == Placement::Flow
            && policy.width == WidthSizing::Content
            && let Some(c) = candidate.content_box.as_mut()
        {
            let width = c.width_mm + 2.0 * policy.padding_mm;
            c.offset_x_mm +=
                (before.frame.width_mm - width) / 2.0 * if candidate.flip_h { -1.0 } else { 1.0 };
            candidate.frame.width_mm = width;
        }
        let Some((dx, dy, w, h)) = content_extent(candidate) else {
            return Ok(false);
        };
        let pad = policy.padding_mm;
        let tape_h = self.printable_height_mm();
        let (x, _, _, _) = self
            .element_rect_mm(index)
            .ok_or("Object geometry is unavailable")?;
        let y = candidate.frame.y_mm;
        let fh = candidate.frame.height_mm;
        let fw = candidate.frame.width_mm;
        if policy.placement == Placement::Free {
            let c = candidate.content_box.as_ref().unwrap();
            let cx = before.frame.x_mm
                + before.frame.width_mm / 2.0
                + c.offset_x_mm * if candidate.flip_h { -1.0 } else { 1.0 };
            let cy = before.frame.y_mm
                + before.frame.height_mm / 2.0
                + c.offset_y_mm * if candidate.flip_v { -1.0 } else { 1.0 };
            let new_w = w + pad * 2.0;
            let new_h = h + pad * 2.0;
            if new_h > tape_h + 1e-7 {
                return Err("Content exceeds printable height — shrink it or use Fit inside");
            }
            candidate.frame.width_mm = new_w;
            candidate.frame.height_mm = new_h;
            let requested_x = cx - new_w / 2.0;
            let requested_y = cy - new_h / 2.0;
            candidate.frame.x_mm = requested_x.max(0.0);
            candidate.frame.y_mm = (cy - new_h / 2.0).clamp(0.0, (tape_h - new_h).max(0.0));
            let c = candidate.content_box.as_mut().unwrap();
            c.offset_x_mm = 0.0;
            c.offset_y_mm = 0.0;
            return Ok((candidate.frame.x_mm - requested_x).abs() > 1e-9
                || (candidate.frame.y_mm - requested_y).abs() > 1e-9);
        }
        if !policy.constrain {
            return Ok(false);
        }
        let fw = if policy.width == WidthSizing::Content {
            candidate.content_box.as_ref().unwrap().width_mm + pad * 2.0
        } else {
            fw
        };
        if w + pad * 2.0 > fw + 1e-7 || h + pad * 2.0 > fh + 1e-7 {
            return Err(
                "Content exceeds its allocation — shrink it, enlarge the allocation or allow overflow",
            );
        }
        let left = (-fw / 2.0 + pad).max(-x - fw / 2.0);
        let top = (-fh / 2.0 + pad).max(-y - fh / 2.0);
        let bottom = (fh / 2.0 - pad).min(tape_h - y - fh / 2.0);
        let c = candidate.content_box.as_mut().unwrap();
        let adjusted_x = dx.clamp(left, fw / 2.0 - pad - w) - dx;
        let adjusted_y = dy.clamp(top, (bottom - h).max(top)) - dy;
        c.offset_x_mm += adjusted_x * if candidate.flip_h { -1.0 } else { 1.0 };
        c.offset_y_mm += adjusted_y * if candidate.flip_v { -1.0 } else { 1.0 };
        Ok(adjusted_x.abs() > 1e-9 || adjusted_y.abs() > 1e-9)
    }

    pub fn selected_canvas_target(&self) -> Option<CanvasTarget> {
        self.selected_element
            .filter(|index| *index < self.elements.len())
            .map(|element| CanvasTarget {
                element,
                edit: self.canvas_edit_target,
            })
    }

    pub fn transform_feedback(&mut self, clamped: bool, free: bool) {
        if clamped {
            self.status_message = if free {
                "At printable edge"
            } else {
                "At allocation edge"
            }
            .into();
        } else if self.status_message.starts_with("At printable edge")
            || self.status_message.starts_with("At allocation edge")
            || self.status_message.starts_with("Content exceeds")
        {
            self.status_message = "Ready".into();
        }
    }

    /// Selecting a target is UI state only: never detach auto-fit content,
    /// upgrade the file format, or request a different rendered bitmap.
    pub fn select_canvas_target(&mut self, target: Option<CanvasTarget>) {
        let target = target.filter(|target| target.element < self.elements.len());
        if self.selected_canvas_target() != target {
            self.selection_revision = self.selection_revision.wrapping_add(1);
        }
        self.selected_element = target.map(|target| target.element);
        if let Some(target) = target {
            self.canvas_edit_target = target.edit;
        }
    }

    /// Only the consecutive relative slots depend on this slot's right edge.
    /// An absolute slot or an unboxed legacy element ends this chain.
    pub fn slot_followers(&self, index: usize) -> Vec<usize> {
        if index >= self.elements.len() {
            return Vec::new();
        }
        self.elements
            .iter()
            .enumerate()
            .skip(index + 1)
            .take_while(|(_, element)| {
                slot_frame(element).is_some_and(|frame| frame.position == PositionMode::Relative)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Show the direct predecessor of a relative slot and its dependent chain,
    /// excluding the target itself. Absolute predecessors remain anchors.
    pub fn slot_related(&self, index: usize) -> Vec<usize> {
        let Some(element) = self.elements.get(index) else {
            return Vec::new();
        };
        let mut related = Vec::new();
        if index > 0
            && slot_frame(element).is_some_and(|frame| frame.position == PositionMode::Relative)
        {
            related.push(index - 1);
        }
        related.extend(self.slot_followers(index));
        related
    }

    /// An interaction hint independent of save/error/printer status messages.
    pub fn canvas_hint(&self) -> Option<String> {
        let valid = |target: &CanvasTarget| target.element < self.elements.len();
        let (phase, target) = if let Some(target) = self.canvas_drag_target.filter(valid) {
            ("Dragging", target)
        } else if let Some(target) = self.canvas_hover_target.filter(valid) {
            ("Candidate", target)
        } else {
            ("Selected", self.selected_canvas_target()?)
        };
        let name = match target.edit {
            CanvasEditTarget::Content => "Content",
            CanvasEditTarget::Slot => "Layout",
        };
        let free = self.elements[target.element].as_object().is_some_and(|o| {
            o.layout
                .as_ref()
                .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Free)
        });
        let consequence = match target.edit {
            CanvasEditTarget::Content if free => "Move, resize or rotate this object.".into(),
            CanvasEditTarget::Content => "Move, resize or rotate content.".to_string(),
            CanvasEditTarget::Slot if free => {
                "Free positioning; use the blue object handles.".into()
            }
            CanvasEditTarget::Slot => {
                let count = self.slot_followers(target.element).len();
                format!(
                    "Horizontal move affects {count} following {}.",
                    if count == 1 { "object" } else { "objects" }
                )
            }
        };
        let action = if !self.is_free_layout() {
            " Legacy layout: convert to free layout before dragging."
        } else if self.overall_flip_h {
            " Turn off label horizontal mirroring to drag."
        } else if phase == "Candidate" {
            " Click to select."
        } else if phase == "Selected" && target.edit == CanvasEditTarget::Slot && !free {
            " Drag to reorder; use handles for size and gap."
        } else {
            ""
        };
        Some(format!(
            "{phase} object {} · {name}: {consequence}{action}",
            target.element + 1
        ))
    }

    pub fn target_dpi(&self) -> u16 {
        if self.printer_connected || self.preview_uses_printer {
            self.printer_dpi
        } else {
            self.design_dpi
        }
    }

    pub fn document(&self) -> LabelDocument {
        // A canvas edit can introduce new fields before the stored version is
        // updated. Render and save must both see the minimum required version.
        let version = self
            .elements
            .iter()
            .fold(self.document_version, |version, element| {
                version.max(element.minimum_version())
            });
        LabelDocument {
            version,
            tape_width_mm: self.tape_width_mm,
            dpi: self.design_dpi,
            font_name: self.font_name.clone(),
            font_margin: self.font_margin,
            flip_h: self.overall_flip_h,
            flip_v: self.overall_flip_v,
            elements: self.elements.clone(),
        }
    }

    pub fn is_free_layout(&self) -> bool {
        self.document_version >= 3
            && self
                .elements
                .iter()
                .all(|element| element.as_object().is_some())
    }

    /// Resolve the unrotated physical object box without changing old files.
    /// Auto-fit objects use the renderer's current geometry on their first edit.
    pub fn resolved_content_box(&self, index: usize) -> Option<ContentBox> {
        let object = self.elements.get(index)?.as_object()?;
        object
            .content_box
            .clone()
            .or_else(|| {
                self.preview_content_bounds
                    .iter()
                    .find(|geometry| geometry.element_index == index)
                    .map(|geometry| geometry.edit_box.clone())
            })
            .or_else(|| Some(content_box_for_frame(&object.frame)))
    }

    /// Explicitly detach object sizing from its slot on the first transform.
    /// Opening, viewing, and converting a legacy file never call this method.
    pub fn pin_content_box(&mut self, index: usize) -> Option<ContentBox> {
        let resolved = self.resolved_content_box(index)?;
        let object = self.elements.get_mut(index)?.as_object_mut()?;
        let changed = object.content_box.is_none();
        object.content_box = Some(resolved.clone());
        self.document_version = self.document_version.max(4);
        if changed {
            self.mark_dirty();
        }
        Some(resolved)
    }

    /// Resolve a model rectangle before whole-label mirroring. Millimeter
    /// frames do not depend on the previous preview or raster rounding.
    pub fn element_rect_mm(&self, index: usize) -> Option<(f64, f64, f64, f64)> {
        let mut cursor = 0.0;
        for (i, element) in self.elements.iter().enumerate().take(index + 1) {
            let frame = slot_frame(element);
            let rect = if let Some(frame) = frame {
                let x = match frame.position {
                    PositionMode::Absolute => frame.x_mm,
                    PositionMode::Relative => cursor + frame.gap_mm,
                };
                Some((x, frame.y_mm, frame.width_mm, frame.height_mm))
            } else {
                self.preview_bounds
                    .iter()
                    .find(|bound| bound.element_index == i)
                    .map(|bound| {
                        let x = if self.overall_flip_h {
                            self.preview_bitmap.as_ref().map_or(bound.x, |bitmap| {
                                bitmap.width().saturating_sub(bound.x + bound.width)
                            })
                        } else {
                            bound.x
                        };
                        let y = if self.overall_flip_v {
                            self.tape_width_px.saturating_sub(bound.y + bound.height)
                        } else {
                            bound.y
                        };
                        let mm = 25.4 / f64::from(self.target_dpi());
                        (
                            f64::from(x) * mm,
                            f64::from(y) * mm,
                            f64::from(bound.width) * mm,
                            f64::from(bound.height) * mm,
                        )
                    })
            };
            if let Some((x, _, width, _)) = rect {
                cursor = x + width;
            }
            if i == index {
                return rect;
            }
        }
        None
    }

    pub fn printable_height_mm(&self) -> f64 {
        f64::from(self.tape_width_px) * 25.4 / f64::from(self.target_dpi())
    }

    /// New objects are independent absolute frames, placed after the furthest
    /// existing right edge with a 3 mm gap. This also works before a rerender.
    pub fn default_object_frame(&self, width_mm: f64, height_mm: f64) -> TextBox {
        let right = (0..self.elements.len())
            .filter_map(|index| self.element_rect_mm(index))
            .map(|(x, _, width, _)| x + width)
            .reduce(f64::max);
        TextBox {
            width_mm,
            height_mm: height_mm.min(self.printable_height_mm()),
            x_mm: right.map_or(0.0, |right| right + 3.0),
            ..TextBox::default()
        }
    }

    fn add_element(&mut self, mut element: LabelElement) {
        use ptouch_render::object_layout::{Placement, WidthSizing};
        let insertion = self.selected_element.filter(|i| {
            self.elements
                .get(*i)
                .and_then(|e| e.as_object())
                .is_some_and(|o| {
                    o.frame.position == PositionMode::Relative
                        || o.layout
                            .as_ref()
                            .is_some_and(|p| p.placement == Placement::Flow)
                })
                || !self.slot_followers(*i).is_empty()
        });
        if let Some(source) = insertion.and_then(|i| self.elements[i].as_object())
            && let Some(new) = element.as_object_mut()
        {
            new.frame.position = PositionMode::Relative;
            new.frame.gap_mm = if source.frame.position == PositionMode::Relative {
                source.frame.gap_mm
            } else {
                insertion
                    .and_then(|i| self.elements.get(i + 1))
                    .and_then(|e| e.as_object())
                    .filter(|o| o.frame.position == PositionMode::Relative)
                    .map_or(3.0, |o| o.frame.gap_mm)
            };
            new.frame.height_mm = source.frame.height_mm;
            new.frame.y_mm = source.frame.y_mm;
            if let Some(policy) = &mut new.layout {
                policy.placement = Placement::Flow;
                policy.width = source
                    .layout
                    .as_ref()
                    .map_or(WidthSizing::Fixed, |p| p.width);
                policy.padding_mm = source.layout.as_ref().map_or(0.0, |p| p.padding_mm);
            }
            if new
                .layout
                .as_ref()
                .is_some_and(|p| p.width == WidthSizing::Fixed)
            {
                new.frame.width_mm = source.frame.width_mm;
                let pad = new.layout.as_ref().map_or(0.0, |p| p.padding_mm) * 2.0;
                if let Some(c) = &mut new.content_box {
                    let scale = ((new.frame.width_mm - pad) / c.width_mm)
                        .min((new.frame.height_mm - pad) / c.height_mm)
                        .clamp(0.01, 1.0);
                    c.width_mm *= scale;
                    c.height_mm *= scale;
                }
            }
        }
        if let Some(object) = element.as_object_mut() {
            ptouch_render::object_layout::update_allocation(object);
        }
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.document_version = self.document_version.max(element.minimum_version());
        let index = insertion.map_or(self.elements.len(), |i| i + 1);
        self.elements.insert(index, element);
        self.select_canvas_target(Some(CanvasTarget {
            element: index,
            edit: CanvasEditTarget::Content,
        }));
        self.mark_dirty();
    }

    /// Insert sequence copies after the source, reusing their gap. Free copies
    /// append after the furthest right edge and at the end of the element list,
    /// preserving existing coordinates and implicit relative bindings.
    pub fn duplicate_element(&mut self, index: usize) -> bool {
        let sequence = !self.slot_followers(index).is_empty()
            || self.flow_chain(index).is_some()
            || self
                .elements
                .get(index)
                .and_then(|e| e.as_object())
                .and_then(|o| o.layout.as_ref())
                .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Flow);
        let Some(mut copy) = self.elements.get(index).cloned() else {
            return false;
        };
        let mut insertion = index + 1;
        let mut free_copy = false;
        let frame = match &mut copy {
            LabelElement::Object(object) => Some(&mut object.frame),
            LabelElement::Text { text_box, .. } => text_box.as_mut(),
            _ => None,
        };
        if let Some(frame) = frame
            && frame.position == PositionMode::Absolute
        {
            if sequence {
                frame.position = PositionMode::Relative;
                frame.gap_mm = self
                    .elements
                    .get(index + 1)
                    .and_then(slot_frame)
                    .filter(|f| f.position == PositionMode::Relative)
                    .map_or(3.0, |f| f.gap_mm);
            } else {
                frame.x_mm = self
                    .default_object_frame(frame.width_mm, frame.height_mm)
                    .x_mm;
                insertion = self.elements.len();
                free_copy = true;
            }
        }
        if sequence
            && let Some(object) = copy.as_object_mut()
            && let Some(policy) = &mut object.layout
            && policy.placement == ptouch_render::object_layout::Placement::Free
        {
            policy.placement = ptouch_render::object_layout::Placement::Flow;
            policy.width = ptouch_render::object_layout::WidthSizing::Fixed;
        }
        let edit = if self.selected_element == Some(index) {
            self.canvas_edit_target
        } else {
            CanvasEditTarget::Content
        };
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.elements.insert(insertion, copy);
        self.select_canvas_target(Some(CanvasTarget {
            element: insertion,
            edit,
        }));
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.status_message = if free_copy {
            "Duplicated at end of label"
        } else {
            "Duplicated after source"
        }
        .into();
        self.mark_dirty();
        true
    }

    pub fn duplicate_selected(&mut self) -> bool {
        self.selected_element
            .is_some_and(|index| self.duplicate_element(index))
    }

    /// Removing a sequence's first object transfers its anchor to the next
    /// object, rather than binding that object to a preceding independent row.
    pub fn delete_element(&mut self, index: usize) -> bool {
        let Some(element) = self.elements.get(index) else {
            return false;
        };
        let anchor = slot_frame(element)
            .filter(|frame| {
                frame.position == PositionMode::Absolute
                    || index == 0
                    || slot_frame(&self.elements[index - 1]).is_none()
            })
            .cloned();
        self.elements.remove(index);
        if let Some(anchor) = anchor
            && let Some(next) = self.elements.get_mut(index).and_then(slot_frame_mut)
            && next.position == PositionMode::Relative
        {
            next.position = anchor.position;
            next.x_mm = anchor.x_mm;
            next.gap_mm = anchor.gap_mm;
        }
        if let Some(selected) = &mut self.selected_element
            && *selected > index
        {
            *selected -= 1;
        }
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.validate_selection();
        self.status_message = "Deleted object".into();
        self.mark_dirty();
        true
    }

    pub fn add_text(&mut self) {
        let element = if self.is_free_layout() {
            let mut object = LabelObject::text(
                "Label".to_string(),
                self.default_object_frame(10.0, self.printable_height_mm()),
            );
            object.content_box = Some(content_box_for_frame(&object.frame));
            object.content_box.as_mut().unwrap().height_mm *= 0.8;
            object.layout = Some(Default::default());
            object.frame.y_mm = (self.printable_height_mm()
                - object.content_box.as_ref().unwrap().height_mm
                - object.layout.as_ref().unwrap().padding_mm * 2.0)
                .max(0.0)
                / 2.0;
            LabelElement::Object(object)
        } else {
            LabelElement::Text {
                content: "Label".to_string(),
                font_size: None,
                align: TextAlign::Left,
                rotation: 0.0,
                flip_h: false,
                flip_v: false,
                text_box: None,
            }
        };
        self.add_element(element);
    }

    pub fn add_image_bytes(&mut self, path: Option<std::path::PathBuf>, bytes: Vec<u8>) -> bool {
        let element = if self.is_free_layout() {
            let mut object = LabelObject::image_from_bytes(
                path,
                bytes,
                self.default_object_frame(10.0, self.printable_height_mm()),
            );
            let ObjectContent::Image {
                bitmap: Some(bitmap),
                ..
            } = &object.content
            else {
                return false;
            };
            // Fit the original aspect ratio inside a useful initial footprint.
            let aspect = f64::from(bitmap.width()) / f64::from(bitmap.height().max(1));
            let height = (self.printable_height_mm() * 0.8).min(40.0 / aspect);
            object.frame.width_mm = (height * aspect).max(25.4 / f64::from(self.target_dpi()));
            object.frame.height_mm = height.max(25.4 / f64::from(self.target_dpi()));
            object.frame.y_mm = (self.printable_height_mm() - object.frame.height_mm) / 2.0;
            object.content_box = Some(content_box_for_frame(&object.frame));
            object.layout = Some(Default::default());
            LabelElement::Object(object)
        } else {
            let element = LabelElement::image_from_bytes(path, bytes);
            if matches!(element, LabelElement::Image { bitmap: None, .. }) {
                return false;
            }
            element
        };
        self.add_element(element);
        true
    }

    pub fn add_line(&mut self) {
        if !self.is_free_layout() {
            return;
        }
        let thickness = 0.3_f64.max(25.4 / f64::from(self.target_dpi()));
        let length = (self.printable_height_mm() * 0.8).min(20.0).max(thickness);
        let mut frame = self.default_object_frame(length, thickness);
        frame.y_mm = (self.printable_height_mm() - thickness).max(0.0) / 2.0;
        self.add_element(LabelElement::Object(LabelObject {
            border: None,
            content_box: Some(content_box_for_frame(&frame)),
            layout: Some(ptouch_render::object_layout::LayoutOptions {
                width: ptouch_render::object_layout::WidthSizing::Fixed,
                ..Default::default()
            }),
            frame,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            content: ObjectContent::Line {
                pattern: Default::default(),
            },
        }));
    }

    pub fn add_cut_mark(&mut self) {
        let element = if self.is_free_layout() {
            let frame = self.default_object_frame(1.27, self.printable_height_mm());
            LabelElement::Object(LabelObject {
                border: None,
                content_box: Some(content_box_for_frame(&frame)),
                layout: Some(ptouch_render::object_layout::LayoutOptions {
                    padding_mm: 0.0,
                    ..Default::default()
                }),
                frame,
                rotation: 0.0,
                flip_h: false,
                flip_v: false,
                content: ObjectContent::CutMark,
            })
        } else {
            LabelElement::CutMark
        };
        self.add_element(element);
    }

    /// Conversion is explicit: opening or saving an older label never changes
    /// its flow layout. The shared renderer resolves positions for all callers.
    pub fn convert_to_free_layout(&mut self) -> ptouch_render::Result<()> {
        let mut document = self.document();
        document.convert_to_free_layout(
            self.tape_width_px,
            self.target_dpi(),
            &mut TextRenderer::new(),
        )?;
        self.document_version = document.version;
        self.elements = document.elements;
        self.mark_dirty();
        Ok(())
    }

    pub fn is_printer_busy(&self) -> bool {
        self.operation_in_progress || self.connecting
    }

    /// Update the tape width in pixels based on the current tape_width_mm
    /// and the connected printer's resolution.
    pub fn update_tape_pixels(&mut self) {
        if let Some(tape) = ptouch_core::tape::find_tape(self.tape_width_mm, self.target_dpi()) {
            let px = u32::from(tape.pixels);
            self.tape_width_px = if (self.printer_connected || self.preview_uses_printer)
                && self.printer_max_px > 0
            {
                px.min(u32::from(self.printer_max_px))
            } else {
                px
            };
        }
    }

    /// Mark the preview as needing re-render.
    pub fn mark_dirty(&mut self) {
        self.edit_revision = self.edit_revision.wrapping_add(1);
        self.needs_rerender = true;
    }

    pub fn history_step(&mut self, redo: bool) {
        if let Some(snapshot) = self.history.step(redo) {
            self.restore_document(snapshot.document);
            self.selected_element = snapshot.selection;
            self.status_message = if redo { "Redo" } else { "Undo" }.into();
        }
    }

    pub fn restore_document(&mut self, doc: LabelDocument) {
        let same_content = self.elements.len() == doc.elements.len()
            && self.elements.iter().zip(&doc.elements).all(|(a, b)| {
                match (a.as_object(), b.as_object()) {
                    (Some(a), Some(b)) => a.content == b.content,
                    _ => false,
                }
            });
        let generation = self.editor_generation;
        self.document_version = doc.version;
        self.elements = doc.elements;
        self.tape_width_mm = doc.tape_width_mm;
        self.design_dpi = doc.dpi;
        self.font_name = doc.font_name;
        self.font_margin = doc.font_margin;
        self.overall_flip_h = doc.flip_h;
        self.overall_flip_v = doc.flip_v;
        self.update_tape_pixels();
        self.validate_selection();
        if same_content {
            self.editor_generation = generation;
        }
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.mark_dirty();
    }

    /// Ensure the selected element index is valid.
    pub fn validate_selection(&mut self) {
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.selection_revision = self.selection_revision.wrapping_add(1);
        if let Some(idx) = self.selected_element
            && idx >= self.elements.len()
        {
            self.selected_element = if self.elements.is_empty() {
                None
            } else {
                Some(self.elements.len() - 1)
            };
        }
    }
}

/// Both v2 text boxes and newer objects define physical template slots.
fn slot_frame(element: &LabelElement) -> Option<&TextBox> {
    match element {
        LabelElement::Object(object) => Some(&object.frame),
        LabelElement::Text { text_box, .. } => text_box.as_ref(),
        _ => None,
    }
}

fn slot_frame_mut(element: &mut LabelElement) -> Option<&mut TextBox> {
    match element {
        LabelElement::Object(object) => Some(&mut object.frame),
        LabelElement::Text { text_box, .. } => text_box.as_mut(),
        _ => None,
    }
}

fn content_box_for_frame(frame: &TextBox) -> ContentBox {
    ContentBox {
        width_mm: frame.width_mm,
        height_mm: frame.height_mm,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_printed_lines_duplicate_with_their_pattern_and_mirrors() {
        let mut state = AppState::default();
        state.add_line();
        let line = state.elements[0].as_object_mut().unwrap();
        line.flip_h = true;
        line.flip_v = true;
        let ObjectContent::Line { pattern } = &mut line.content else {
            panic!("printed line")
        };
        pattern.kind = ptouch_render::stroke::PatternKind::Dashed;
        let original = line.clone();
        assert!(state.duplicate_selected());
        let copy = state.elements[1].as_object().unwrap();
        assert_eq!(copy.content, original.content);
        assert!(copy.flip_h && copy.flip_v);
        assert!(copy.frame.x_mm >= original.frame.x_mm + original.frame.width_mm);
        let document = state.document();
        assert_eq!(document.version, 9);
        let rendered =
            ptouch_render::document::render_document(&document, 76, 180, &mut TextRenderer::new())
                .unwrap();
        assert_eq!(rendered.bounds.len(), 2);
        assert!(rendered.bitmap.is_some());

        let mut legacy = AppState {
            document_version: 1,
            ..Default::default()
        };
        let before = legacy.document();
        legacy.add_line();
        assert_eq!(
            legacy.document(),
            before,
            "adding a line must not implicitly convert a legacy layout"
        );
    }

    #[test]
    fn selecting_and_hovering_targets_never_pin_or_change_the_saved_layout() {
        let mut state = AppState {
            document_version: 3,
            elements: vec![LabelElement::Object(LabelObject::text(
                "PORT1",
                TextBox::default(),
            ))],
            needs_rerender: false,
            ..AppState::default()
        };
        let before = state.document().to_toml_string().unwrap();
        for edit in [CanvasEditTarget::Content, CanvasEditTarget::Slot] {
            let target = CanvasTarget { element: 0, edit };
            state.select_canvas_target(Some(target));
            assert_eq!(state.selected_canvas_target(), Some(target));
            state.canvas_hover_target = Some(target);
            state.canvas_drag_target = Some(target);
            assert!(state.canvas_hint().unwrap().starts_with("Dragging"));
        }
        state.select_canvas_target(None);
        assert!(state.selected_canvas_target().is_none());
        state.select_canvas_target(Some(CanvasTarget {
            element: 99,
            edit: CanvasEditTarget::Slot,
        }));
        assert!(state.selected_canvas_target().is_none());
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        assert_eq!(state.document_version, 3);
        assert!(!state.needs_rerender);
        assert!(state.elements[0].as_object().unwrap().content_box.is_none());
    }

    #[test]
    fn slot_relationships_stop_at_absolute_slots_and_exclude_the_target() {
        let mut state = AppState::default();
        for position in [
            PositionMode::Absolute,
            PositionMode::Relative,
            PositionMode::Relative,
            PositionMode::Absolute,
            PositionMode::Relative,
        ] {
            state.elements.push(LabelElement::Object(LabelObject::text(
                "",
                TextBox {
                    position,
                    ..TextBox::default()
                },
            )));
        }
        state.elements.push(LabelElement::CutMark);
        state.elements.push(LabelElement::Object(LabelObject::text(
            "",
            TextBox {
                position: PositionMode::Relative,
                ..TextBox::default()
            },
        )));
        assert_eq!(state.slot_followers(0), [1, 2]);
        assert_eq!(state.slot_related(0), [1, 2]);
        assert_eq!(state.slot_related(1), [0, 2]);
        assert_eq!(state.slot_related(2), [1]);
        assert_eq!(state.slot_related(3), [4]);
        assert_eq!(state.slot_related(4), [3]);
        assert_eq!(state.slot_related(6), [5]);
        assert!(state.slot_followers(4).is_empty());
        assert!(state.slot_followers(99).is_empty());
        assert!(state.slot_related(99).is_empty());
    }

    #[test]
    fn v2_text_slot_relationships_and_hints_preserve_the_original_format() {
        let boxed_text = |position| LabelElement::Text {
            content: "PORT".into(),
            font_size: None,
            align: TextAlign::Center,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            text_box: Some(TextBox {
                x_mm: 5.0,
                gap_mm: 3.0,
                position,
                ..TextBox::default()
            }),
        };
        let mut state = AppState {
            document_version: 2,
            elements: vec![
                boxed_text(PositionMode::Absolute),
                boxed_text(PositionMode::Relative),
                boxed_text(PositionMode::Relative),
                LabelElement::Padding { pixels: 20 },
                boxed_text(PositionMode::Relative),
                boxed_text(PositionMode::Absolute),
                boxed_text(PositionMode::Relative),
            ],
            needs_rerender: false,
            ..AppState::default()
        };
        let before = state.document().to_toml_string().unwrap();
        state.select_canvas_target(Some(CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Slot,
        }));
        assert_eq!(state.slot_followers(0), [1, 2]);
        assert_eq!(state.slot_related(0), [1, 2]);
        assert_eq!(state.slot_related(1), [0, 2]);
        assert_eq!(state.slot_related(2), [1]);
        assert!(state.slot_followers(2).is_empty());
        assert_eq!(state.slot_related(4), [3]);
        assert!(state.slot_followers(4).is_empty());
        assert_eq!(state.slot_related(5), [6]);
        assert!(state.canvas_hint().unwrap().contains("2 following objects"));
        assert_eq!(state.element_rect_mm(2), Some((31.0, 0.0, 10.0, 8.0)));
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        assert_eq!(state.document().version, 2);
        assert_eq!(state.document_version, 2);
        assert!(!state.needs_rerender);
        assert!(
            state
                .elements
                .iter()
                .all(|element| element.as_object().is_none())
        );
    }

    #[test]
    fn canvas_hints_prioritize_drag_then_hover_then_selection_and_explain_effects() {
        let mut state = AppState {
            elements: vec![
                LabelElement::Object(LabelObject::text("A", TextBox::default())),
                LabelElement::Object(LabelObject::text(
                    "B",
                    TextBox {
                        position: PositionMode::Relative,
                        ..TextBox::default()
                    },
                )),
            ],
            status_message: "Render error: keep this visible".into(),
            ..AppState::default()
        };
        let selected = CanvasTarget {
            element: 1,
            edit: CanvasEditTarget::Slot,
        };
        let hovered = CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Content,
        };
        let dragged = CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Slot,
        };
        state.select_canvas_target(Some(selected));
        let hint = state.canvas_hint().unwrap();
        assert!(hint.starts_with("Selected object 2 · Layout:"));
        assert!(hint.contains("0 following objects"));
        assert!(hint.contains("Drag to reorder; use handles for size and gap."));
        state.canvas_hover_target = Some(hovered);
        let hint = state.canvas_hint().unwrap();
        assert!(hint.starts_with("Candidate object 1 · Content:"));
        assert!(hint.contains("Move, resize or rotate content"));
        assert!(hint.contains("Click to select."));
        state.canvas_drag_target = Some(dragged);
        let hint = state.canvas_hint().unwrap();
        assert!(hint.starts_with("Dragging object 1 · Layout:"));
        assert!(hint.contains("1 following object"));
        state.overall_flip_h = true;
        assert!(
            state
                .canvas_hint()
                .unwrap()
                .contains("Turn off label horizontal mirroring")
        );
        state.canvas_drag_target = None;
        state.canvas_hover_target = None;
        state.elements[0] = LabelElement::CutMark;
        assert!(state.canvas_hint().unwrap().contains("Legacy layout"));
        assert_eq!(state.status_message, "Render error: keep this visible");
    }

    #[test]
    fn duplicates_insert_into_relative_chains_without_moving_absolute_anchors() {
        let object = |x, position, gap| {
            LabelElement::Object(LabelObject::text(
                "PORT",
                TextBox {
                    x_mm: x,
                    position,
                    gap_mm: gap,
                    ..Default::default()
                },
            ))
        };
        let mut state = AppState {
            elements: vec![
                object(5.0, PositionMode::Absolute, 3.0),
                object(0.0, PositionMode::Relative, 2.8),
                object(100.0, PositionMode::Absolute, 3.0),
            ],
            selected_element: Some(0),
            canvas_edit_target: CanvasEditTarget::Slot,
            ..Default::default()
        };
        assert!(state.duplicate_selected());
        assert_eq!(state.selected_element, Some(1));
        assert_eq!(state.canvas_edit_target, CanvasEditTarget::Slot);
        for (i, x) in [5.0, 17.8, 30.6, 100.0].into_iter().enumerate() {
            assert!((state.element_rect_mm(i).unwrap().0 - x).abs() < 1e-10);
        }
        assert!(state.duplicate_element(2));
        assert!((state.element_rect_mm(3).unwrap().0 - 43.4).abs() < 1e-10);
        assert_eq!(state.element_rect_mm(4).unwrap().0, 100.0);
        assert!(!state.duplicate_element(999));
    }

    #[test]
    fn delete_then_duplicate_free_row_appends_without_overlaps_and_undoes() {
        use ptouch_render::object_layout::{LayoutOptions, WidthSizing};

        for version in [2, 6, 7] {
            let mut state = AppState {
                document_version: version,
                elements: (0..5)
                    .map(|i| {
                        let frame = TextBox {
                            x_mm: f64::from(i) * 17.5,
                            width_mm: 14.5,
                            height_mm: 10.0,
                            ..Default::default()
                        };
                        let content = format!("PORT {i}\nDEVICE {i}");
                        if version == 2 {
                            LabelElement::Text {
                                content,
                                font_size: None,
                                align: TextAlign::Center,
                                rotation: 0.0,
                                flip_h: false,
                                flip_v: false,
                                text_box: Some(frame),
                            }
                        } else {
                            let mut object = LabelObject::text(content, frame);
                            if version == 7 {
                                object.content_box = Some(content_box_for_frame(&object.frame));
                                object.layout = Some(LayoutOptions {
                                    width: WidthSizing::Fixed,
                                    ..Default::default()
                                });
                            }
                            LabelElement::Object(object)
                        }
                    })
                    .collect(),
                ..Default::default()
            };
            let initial = state.document();
            state
                .history
                .observe(initial.clone(), Some(0), false, false, 0.0);
            assert!(state.delete_element(0));
            let deleted = state.document();
            state
                .history
                .observe(deleted.clone(), Some(1), false, false, 1.0);

            assert!(state.duplicate_element(1));
            assert_eq!(state.selected_element, Some(4));
            assert_eq!(&state.elements[..4], deleted.elements.as_slice());
            assert_eq!(state.element_rect_mm(4).unwrap().0, 87.5);
            let mut expected_copy = deleted.elements[1].clone();
            match &mut expected_copy {
                LabelElement::Text { text_box, .. } => text_box.as_mut().unwrap().x_mm = 87.5,
                LabelElement::Object(object) => object.frame.x_mm = 87.5,
                _ => unreachable!(),
            }
            assert_eq!(state.elements[4], expected_copy);
            let duplicated = state.document();
            assert_eq!(duplicated.version, version);
            LabelDocument::from_toml_str(&duplicated.to_toml_string().unwrap()).unwrap();
            state
                .history
                .observe(duplicated.clone(), Some(4), false, false, 2.0);

            state.history_step(false);
            assert_eq!(state.document(), deleted);
            state.history_step(false);
            assert_eq!(state.document(), initial);
            state.history_step(true);
            state.history_step(true);
            assert_eq!(state.document(), duplicated);
            assert!(state.reorder_object(4, 1));
            assert_eq!(state.elements[1], expected_copy);
            assert_eq!(state.element_rect_mm(1).unwrap().0, 87.5);
        }
    }

    #[test]
    fn deleting_sequence_anchor_preserves_its_origin_and_next_row() {
        for version in [2, 6, 7] {
            for relative_anchor in [false, true] {
                let mut elements: Vec<_> = (0..5)
                    .map(|i| {
                        LabelElement::Object(LabelObject::text(
                            format!("PORT {i}"),
                            TextBox {
                                position: if i == 1 || i == 2 || i == 0 && relative_anchor {
                                    PositionMode::Relative
                                } else {
                                    PositionMode::Absolute
                                },
                                x_mm: if i == 0 {
                                    5.0
                                } else {
                                    100.0 + f64::from(i) * 20.0
                                },
                                gap_mm: if i == 0 { 5.0 } else { 2.8 },
                                ..Default::default()
                            },
                        ))
                    })
                    .collect();
                if version == 2 {
                    for element in &mut elements {
                        let object = element.as_object().unwrap();
                        *element = LabelElement::Text {
                            content: "PORT".into(),
                            font_size: None,
                            align: TextAlign::Center,
                            rotation: 0.0,
                            flip_h: false,
                            flip_v: false,
                            text_box: Some(object.frame.clone()),
                        };
                    }
                }
                let mut state = AppState {
                    document_version: version,
                    elements,
                    selected_element: Some(2),
                    ..Default::default()
                };
                let first_x = state.element_rect_mm(0).unwrap().0;
                let independent = state.elements[3..].to_vec();
                assert!(!state.delete_element(999));
                assert!(state.delete_element(0));
                assert_eq!(state.selected_element, Some(1));
                assert_eq!(state.element_rect_mm(0).unwrap().0, first_x);
                assert!((state.element_rect_mm(1).unwrap().0 - (first_x + 12.8)).abs() < 1e-9);
                assert_eq!(&state.elements[2..], independent.as_slice());
                assert_eq!(state.document().version, version);
            }
        }
    }

    #[test]
    fn deleting_sequence_after_an_independent_object_keeps_the_anchor() {
        let mut state = AppState {
            elements: vec![
                LabelElement::Object(LabelObject::text("FREE", TextBox::default())),
                LabelElement::Object(LabelObject::text(
                    "A",
                    TextBox {
                        x_mm: 100.0,
                        ..Default::default()
                    },
                )),
                LabelElement::Object(LabelObject::text(
                    "B",
                    TextBox {
                        position: PositionMode::Relative,
                        gap_mm: 2.8,
                        ..Default::default()
                    },
                )),
            ],
            ..Default::default()
        };
        assert!(state.delete_element(1));
        assert_eq!(state.element_rect_mm(1).unwrap().0, 100.0);
        assert!(state.duplicate_element(1));
        // The remaining v3 object is now independent and duplicates at the end.
        assert_eq!(state.element_rect_mm(2).unwrap().0, 113.0);
    }

    #[test]
    fn free_copy_does_not_reanchor_following_legacy_elements() {
        let mut state = AppState {
            document_version: 3,
            elements: vec![
                LabelElement::Object(LabelObject::text("A", TextBox::default())),
                LabelElement::Padding { pixels: 20 },
                LabelElement::Object(LabelObject::text(
                    "B",
                    TextBox {
                        x_mm: 50.0,
                        ..Default::default()
                    },
                )),
            ],
            ..Default::default()
        };
        let before = state.elements.clone();
        assert!(state.duplicate_element(0));
        assert_eq!(&state.elements[..3], before.as_slice());
        assert_eq!(state.selected_element, Some(3));
        assert_eq!(state.element_rect_mm(3).unwrap().0, 63.0);
    }

    #[test]
    fn duplicate_sequence_supports_legacy_boxes_and_free_anchors() {
        use ptouch_render::object_layout::{LayoutOptions, Placement};

        for version in [2, 7] {
            let mut elements = vec![
                LabelElement::Object(LabelObject::text(
                    "A",
                    TextBox {
                        x_mm: 5.0,
                        ..Default::default()
                    },
                )),
                LabelElement::Object(LabelObject::text(
                    "B",
                    TextBox {
                        position: PositionMode::Relative,
                        gap_mm: 2.8,
                        ..Default::default()
                    },
                )),
            ];
            if version == 2 {
                for element in &mut elements {
                    let object = element.as_object().unwrap();
                    let ObjectContent::Text { content, .. } = &object.content else {
                        unreachable!()
                    };
                    *element = LabelElement::Text {
                        content: content.clone(),
                        font_size: None,
                        align: TextAlign::Center,
                        rotation: 0.0,
                        flip_h: false,
                        flip_v: false,
                        text_box: Some(object.frame.clone()),
                    };
                }
            } else {
                let first = elements[0].as_object_mut().unwrap();
                first.content_box = Some(content_box_for_frame(&first.frame));
                first.layout = Some(LayoutOptions::default());
            }
            let mut state = AppState {
                document_version: version,
                elements,
                ..Default::default()
            };
            assert!(state.duplicate_element(0));
            for (i, x) in [5.0, 17.8, 30.6].into_iter().enumerate() {
                assert!((state.element_rect_mm(i).unwrap().0 - x).abs() < 1e-9);
            }
            if version == 7 {
                assert_eq!(
                    state.elements[1]
                        .as_object()
                        .unwrap()
                        .layout
                        .as_ref()
                        .unwrap()
                        .placement,
                    Placement::Flow
                );
            }
            let saved = state.document().to_toml_string().unwrap();
            assert_eq!(
                LabelDocument::from_toml_str(&saved).unwrap().version,
                version
            );
        }
    }

    #[test]
    fn duplicate_all_object_types_keeps_content_transforms_and_embedded_images() {
        let image = LabelObject::image_from_bytes(None, br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#.to_vec(), TextBox::default());
        for content in [
            ObjectContent::Text {
                content: "{{port}}".into(),
                font_size: Some(9.0),
                line_spacing: ptouch_render::text::DEFAULT_LINE_SPACING,
                styles: vec![ptouch_render::styled_text::StyleSpan {
                    start: 0,
                    end: 1,
                    style: ptouch_render::styled_text::TextStyle {
                        font_size: Some(18.0),
                        bold: true,
                        ..Default::default()
                    },
                }],
                align: TextAlign::Right,
            },
            image.content,
            ObjectContent::CutMark,
            ObjectContent::Padding,
        ] {
            let object = LabelObject {
                border: None,
                layout: None,
                frame: TextBox {
                    x_mm: 5.0,
                    ..Default::default()
                },
                content_box: Some(ContentBox {
                    width_mm: 7.0,
                    height_mm: 3.0,
                    offset_x_mm: 0.8,
                    offset_y_mm: -0.2,
                }),
                rotation: 30.0,
                flip_h: true,
                flip_v: true,
                content,
            };
            let mut state = AppState {
                document_version: 4,
                elements: vec![LabelElement::Object(object.clone())],
                selected_element: Some(0),
                ..Default::default()
            };
            assert!(state.duplicate_selected());
            let saved = state.document().to_toml_string().unwrap();
            let restored = LabelDocument::from_toml_str(&saved).unwrap();
            let copy = restored.elements[1].as_object().unwrap();
            assert_eq!(copy.content_box, object.content_box);
            assert_eq!(
                (copy.rotation, copy.flip_h, copy.flip_v),
                (30.0, true, true)
            );
            assert_eq!(copy.frame.x_mm, 18.0);
            let mut normalized = restored.clone();
            normalized.elements[1].as_object_mut().unwrap().frame = object.frame.clone();
            let mut first = normalized.clone();
            first.elements.truncate(1);
            normalized.elements.remove(0);
            assert_eq!(
                normalized.to_toml_string().unwrap(),
                first.to_toml_string().unwrap()
            );
            if let ObjectContent::Image {
                bitmap, image_data, ..
            } = &copy.content
            {
                assert!(bitmap.is_some());
                assert!(!image_data.is_empty());
            }
        }
    }

    #[test]
    fn duplication_keeps_legacy_versions_and_does_not_pin_auto_fit_objects() {
        let legacy = LabelElement::Text {
            content: "PORT".into(),
            font_size: None,
            align: TextAlign::Center,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            text_box: None,
        };
        let mut boxed = legacy.clone();
        if let LabelElement::Text { text_box, .. } = &mut boxed {
            *text_box = Some(TextBox {
                x_mm: 5.0,
                ..Default::default()
            });
        }
        for (version, elements) in [
            (
                1,
                vec![
                    legacy,
                    LabelElement::CutMark,
                    LabelElement::Padding { pixels: 20 },
                ],
            ),
            (2, vec![boxed]),
            (
                3,
                vec![LabelElement::Object(LabelObject::text(
                    "PORT",
                    TextBox::default(),
                ))],
            ),
        ] {
            for index in 0..elements.len() {
                let mut state = AppState {
                    document_version: version,
                    elements: elements.clone(),
                    selected_element: Some(index),
                    ..Default::default()
                };
                assert!(state.duplicate_selected());
                let restored =
                    LabelDocument::from_toml_str(&state.document().to_toml_string().unwrap())
                        .unwrap();
                assert_eq!(restored.version, version);
                assert_eq!(restored.elements.len(), elements.len() + 1);
                if let Some(object) = restored.elements[index + 1].as_object() {
                    assert!(object.content_box.is_none());
                }
            }
        }
    }

    #[test]
    fn line_spacing_upgrades_render_and_save_together_and_survives_duplication() {
        let mut state = AppState::default();
        let mut old = LabelObject::text("Label", TextBox::default());
        old.content_box = Some(content_box_for_frame(&old.frame));
        state.elements.push(LabelElement::Object(old));
        state.selected_element = Some(0);
        assert_eq!(state.document().version, 4);
        if let ObjectContent::Text { line_spacing, .. } =
            &mut state.elements[0].as_object_mut().unwrap().content
        {
            *line_spacing = 0.9;
        }
        assert_eq!(state.document().version, 5);
        assert!(state.duplicate_selected());
        let saved = state.document().to_toml_string().unwrap();
        let restored = LabelDocument::from_toml_str(&saved).unwrap();
        assert_eq!(restored.version, 5);
        for element in &restored.elements {
            assert!(
                matches!(element.as_object().unwrap().content, ObjectContent::Text { line_spacing, .. } if line_spacing == 0.9)
            );
        }
        for element in &mut state.elements {
            if let ObjectContent::Text { line_spacing, .. } =
                &mut element.as_object_mut().unwrap().content
            {
                *line_spacing = ptouch_render::text::DEFAULT_LINE_SPACING;
            }
        }
        assert_eq!(state.document().version, 4);
        assert!(
            !state
                .document()
                .to_toml_string()
                .unwrap()
                .contains("line_spacing")
        );
    }

    #[test]
    fn new_objects_have_independent_frames_after_the_furthest_right_edge() {
        let mut state = AppState::default();
        assert_eq!(state.document_version, DEFAULT_DOCUMENT_VERSION);
        state.add_text();
        state.add_text();
        let first = state.elements[0].as_object_mut().unwrap();
        first.frame.x_mm = 40.0;
        state.add_text();
        let width = state.elements[0].as_object().unwrap().frame.width_mm;
        for element in &state.elements {
            let object = element.as_object().unwrap();
            let content = object.content_box.as_ref().unwrap();
            assert!((content.height_mm / state.printable_height_mm() - 0.8).abs() < 1e-9);
            assert_eq!(object.frame.height_mm, content.height_mm);
            assert_eq!(object.frame.position, PositionMode::Absolute);
        }
        assert_eq!(state.element_rect_mm(1).unwrap().0, width + 3.0);
        assert_eq!(state.element_rect_mm(2).unwrap().0, 40.0 + width + 3.0);
        assert!(state.needs_rerender);
    }

    #[test]
    fn mixed_v3_documents_still_offer_explicit_conversion() {
        let mut state = AppState::default();
        state.add_text();
        state.elements.push(LabelElement::CutMark);
        assert!(!state.is_free_layout());
        state.convert_to_free_layout().unwrap();
        assert!(state.is_free_layout());
    }

    #[test]
    fn relative_frames_resolve_in_millimeters_without_preview_rounding() {
        let mut state = AppState::default();
        for _ in 0..300 {
            let frame = TextBox {
                position: PositionMode::Relative,
                gap_mm: 3.0,
                ..TextBox::default()
            };
            state
                .elements
                .push(LabelElement::Object(LabelObject::text("", frame)));
        }
        assert!(state.preview_bounds.is_empty());
        assert_eq!(state.element_rect_mm(299), Some((3890.0, 0.0, 10.0, 8.0)));
        assert_eq!(state.default_object_frame(10.0, 8.0).x_mm, 3903.0);
    }

    #[test]
    fn initial_image_frame_fits_tape_preserves_aspect_and_embeds_bytes() {
        use std::io::Cursor;
        let source = image::GrayImage::from_pixel(200, 100, image::Luma([0]));
        let mut bytes = Cursor::new(Vec::new());
        source
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        let mut state = AppState::default();
        assert!(state.add_image_bytes(None, bytes.clone()));
        let object = state.elements[0].as_object().unwrap();
        let content = object.content_box.as_ref().unwrap();
        assert!((content.width_mm / content.height_mm - 2.0).abs() < 1e-9);
        assert!((object.frame.height_mm - state.printable_height_mm() * 0.8).abs() < 1e-9);
        let ObjectContent::Image {
            image_data, bitmap, ..
        } = &object.content
        else {
            panic!()
        };
        assert_eq!(*image_data, bytes);
        assert!(bitmap.is_some());
        let count = state.elements.len();
        assert!(!state.add_image_bytes(None, b"invalid image".to_vec()));
        assert_eq!(state.elements.len(), count);
    }

    #[test]
    fn every_new_object_fits_narrow_tape_height() {
        let mut state = AppState {
            tape_width_px: 32,
            tape_width_mm: 6,
            ..AppState::default()
        };
        state.add_text();
        state.add_cut_mark();
        for element in &state.elements {
            let object = element.as_object().unwrap();
            let frame = &object.frame;
            let content = object.content_box.as_ref().unwrap();
            assert!(content.height_mm <= frame.height_mm);
            assert!(frame.height_mm <= state.printable_height_mm());
            assert!(frame.y_mm + frame.height_mm <= state.printable_height_mm() + 1e-9);
        }
    }

    #[test]
    fn first_object_edit_pins_preview_geometry_without_changing_its_slot() {
        let slot = TextBox {
            x_mm: 5.0,
            ..TextBox::default()
        };
        let mut state = AppState {
            document_version: 3,
            elements: vec![LabelElement::Object(LabelObject::text(
                "PORT1",
                slot.clone(),
            ))],
            needs_rerender: false,
            ..AppState::default()
        };
        assert_eq!(state.canvas_edit_target, CanvasEditTarget::Content);
        assert_eq!(
            state.resolved_content_box(0),
            Some(content_box_for_frame(&slot))
        );
        let actual = ContentBox {
            width_mm: 8.7,
            height_mm: 3.9,
            offset_x_mm: -0.2,
            offset_y_mm: 0.1,
        };
        state.preview_content_bounds.push(ContentGeometry {
            element_index: 0,
            corners: [[0.0; 2]; 4],
            edit_box: actual.clone(),
        });
        assert_eq!(state.resolved_content_box(0), Some(actual.clone()));
        assert_eq!(state.document_version, 3);
        assert!(!state.needs_rerender);
        assert_eq!(state.pin_content_box(0), Some(actual.clone()));
        assert_eq!(state.document_version, 4);
        assert!(state.needs_rerender);
        assert_eq!(state.elements[0].as_object().unwrap().frame, slot);
        // Once manual, a stale preview must never replace the stored geometry.
        state.preview_content_bounds[0].edit_box.width_mm = 1.0;
        assert_eq!(state.resolved_content_box(0), Some(actual));
        assert!(state.resolved_content_box(1).is_none());
    }

    #[test]
    fn manual_geometry_upgrades_document_and_survives_save_without_moving_slot() {
        let mut state = AppState {
            document_version: 3,
            ..AppState::default()
        };
        let mut object = LabelObject::text("PORT1", TextBox::default());
        let content_box = ContentBox {
            width_mm: 12.3,
            height_mm: 4.5,
            offset_x_mm: -1.2,
            offset_y_mm: 0.7,
        };
        object.content_box = Some(content_box.clone());
        object.rotation = 45.0;
        state.elements.push(LabelElement::Object(object));
        let document = state.document();
        assert_eq!(document.version, 4);
        let reopened = LabelDocument::from_toml_str(&document.to_toml_string().unwrap()).unwrap();
        assert_eq!(reopened.version, 4);
        let object = reopened.elements[0].as_object().unwrap();
        assert_eq!(object.content_box, Some(content_box));
        assert_eq!(object.frame, TextBox::default());
        assert_eq!(object.rotation, 45.0);
    }

    #[test]
    fn explicit_legacy_conversion_does_not_enable_independent_object_sizing() {
        let mut state = AppState {
            document_version: 1,
            elements: vec![LabelElement::CutMark, LabelElement::Padding { pixels: 20 }],
            ..AppState::default()
        };
        state.convert_to_free_layout().unwrap();
        assert!(state.is_free_layout());
        assert!(
            state
                .elements
                .iter()
                .all(|element| element.as_object().unwrap().content_box.is_none())
        );
    }
    fn small_free(text: &str, x: f64, y: f64) -> LabelElement {
        let mut object = LabelObject::text(
            text,
            TextBox {
                width_mm: 4.0,
                height_mm: 2.0,
                x_mm: x,
                y_mm: y,
                ..Default::default()
            },
        );
        object.content_box = Some(ContentBox {
            width_mm: 4.0,
            height_mm: 2.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
        object.layout = Some(ptouch_render::object_layout::LayoutOptions {
            width: ptouch_render::object_layout::WidthSizing::Fixed,
            ..Default::default()
        });
        LabelElement::Object(object)
    }

    #[test]
    fn free_geometry_has_no_hidden_allocation_and_clamps_at_printable_edge() {
        let state = AppState {
            document_version: 7,
            elements: vec![small_free("A", 2.0, 1.0)],
            ..Default::default()
        };
        let before = state.elements[0].as_object().unwrap();
        let mut rotated = before.clone();
        rotated.rotation = 90.0;
        assert!(!state.constrain_transform(0, before, &mut rotated).unwrap());
        assert!((rotated.frame.width_mm - 2.0).abs() < 1e-9);
        assert!((rotated.frame.height_mm - 4.0).abs() < 1e-9);
        assert!((rotated.frame.x_mm - 3.0).abs() < 1e-9);
        assert!(rotated.frame.y_mm.abs() < 1e-9);
        let mut moved = before.clone();
        moved.content_box.as_mut().unwrap().offset_x_mm = -20.0;
        moved.content_box.as_mut().unwrap().offset_y_mm = -20.0;
        assert!(state.constrain_transform(0, before, &mut moved).unwrap());
        assert_eq!((moved.frame.x_mm, moved.frame.y_mm), (0.0, 0.0));
        assert_eq!(
            (
                moved.content_box.as_ref().unwrap().offset_x_mm,
                moved.content_box.as_ref().unwrap().offset_y_mm
            ),
            (0.0, 0.0)
        );
    }

    #[test]
    fn swapping_positions_and_template_content_are_distinct_operations() {
        let mut state = AppState {
            document_version: 7,
            elements: vec![small_free("A", 2.0, 1.0), small_free("B", 20.0, 2.0)],
            ..Default::default()
        };
        assert!(state.swap_positions(0, 1));
        assert_eq!(state.elements[0].as_object().unwrap().frame.x_mm, 20.0);
        assert!(
            matches!(&state.elements[0].as_object().unwrap().content,ObjectContent::Text{content,..} if content=="A")
        );
        let height = state.printable_height_mm();
        for element in &mut state.elements {
            AppState::use_flow_allocation(element.as_object_mut().unwrap(), height);
        }
        let frames: Vec<_> = state
            .elements
            .iter()
            .map(|e| e.as_object().unwrap().frame.clone())
            .collect();
        let border = ptouch_render::stroke::BorderStyle {
            padding_mm: 0.1,
            ..Default::default()
        };
        state.elements[0].as_object_mut().unwrap().border = Some(border.clone());
        assert!(!state.can_swap_positions(0, 1));
        assert!(state.swap_content(0, 1));
        assert!(state.elements[0].as_object().unwrap().border.is_none());
        assert_eq!(state.elements[1].as_object().unwrap().border, Some(border));
        assert_eq!(
            state
                .elements
                .iter()
                .map(|e| e.as_object().unwrap().frame.clone())
                .collect::<Vec<_>>(),
            frames
        );
        assert!(
            matches!(&state.elements[0].as_object().unwrap().content,ObjectContent::Text{content,..} if content=="B")
        );
    }
}
