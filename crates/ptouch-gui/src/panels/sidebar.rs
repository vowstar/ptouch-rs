// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Left sidebar panel: printer info, tape selection, and element list.

use log::info;

use ptouch_core::device::DeviceFlags;
use ptouch_core::protocol::PrintQuality;
use ptouch_core::tape;

use crate::state::{
    AppState, CanvasEditTarget, CanvasTarget, NetworkPrinterCandidate, PrinterCommand,
    PrinterTarget,
};

/// Render the left sidebar.
pub fn show_sidebar(ui: &mut egui::Ui, state: &mut AppState) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        show_printer_section(ui, state);
        ui.separator();
        show_tape_section(ui, state);
        ui.separator();
        show_print_options(ui, state);
        ui.separator();
        show_template_values(ui, state);
        ui.separator();
        show_mirror_section(ui, state);
        ui.separator();
        show_elements_section(ui, state);
    });
}

fn show_template_values(ui: &mut egui::Ui, state: &mut AppState) {
    let placeholders =
        ptouch_render::document::LabelDocument::placeholders_in_elements(&state.elements);
    if placeholders.is_empty() {
        return;
    }
    ui.heading("Template values");
    ui.small("Values apply to preview and printing. Saved layout tokens stay unchanged.");
    for name in placeholders {
        let mut value = state
            .template_values
            .get(&name)
            .cloned()
            .unwrap_or_default();
        let response = ui.horizontal(|ui| {
            ui.label(&name);
            ui.text_edit_singleline(&mut value)
        });
        if response.inner.changed() {
            state.template_values.insert(name, value);
            state.mark_dirty();
        }
    }
    if state.has_unfilled_template_values() {
        ui.colored_label(
            egui::Color32::YELLOW,
            "Fill each value to print the template",
        );
    }
}

/// Printer connection section.
fn show_printer_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Printer");
    ui.add_space(4.0);

    let old_target = state.printer_target.clone();
    ui.add_enabled_ui(!state.is_printer_busy(), |ui| {
        egui::ComboBox::from_label("Connection")
            .selected_text(state.printer_target.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut state.printer_target,
                    crate::state::PrinterTarget::Usb,
                    "USB (automatic)",
                );
                for target in state.usb_targets.iter().chain(&state.bluetooth_targets) {
                    ui.selectable_value(&mut state.printer_target, target.clone(), target.label());
                }
            });
    });
    if state.printer_target != old_target {
        state.calibration_enabled = false;
        state.printer_connected = false;
        state.preview_uses_printer = false;
        state.update_tape_pixels();
        state.mark_dirty();
        state.printer_model = None;
        state.manual_tcp_tape_width_mm = None;
        state.printer_status = Some("Connecting...".to_string());
        state.connecting = true;
        state.auto_cut = !state.printer_target.is_bluetooth();
        if let Some(ref tx) = state.printer_cmd_tx {
            let _ = tx.send(PrinterCommand::Poll(state.printer_target.clone()));
        }
    }

    let mut connect_tcp = false;
    let mut selected_network = None;
    ui.add_enabled_ui(!state.is_printer_busy(), |ui| {
        if !state.network_targets.is_empty() {
            let selected_name = state
                .network_targets
                .iter()
                .find(|candidate| candidate.address.to_string() == state.tcp_address)
                .map(NetworkPrinterCandidate::label)
                .unwrap_or_else(|| "Select discovered printer".to_string());
            egui::ComboBox::from_label("Discovered printers")
                .selected_text(&selected_name)
                .show_ui(ui, |ui| {
                    for candidate in &state.network_targets {
                        if ui
                            .selectable_label(
                                state.tcp_address == candidate.address.to_string(),
                                candidate.label(),
                            )
                            .clicked()
                        {
                            selected_network = Some(candidate.clone());
                        }
                    }
                });
        } else {
            ui.small("No raw TCP printers discovered yet");
        }
        ui.horizontal(|ui| {
            ui.label("TCP address");
            ui.text_edit_singleline(&mut state.tcp_address)
                .on_hover_text("Raw printer socket, usually port 9100");
        });
        egui::ComboBox::from_label("TCP model")
            .selected_text(format!("PID 0x{:04x}", state.tcp_pid))
            .show_ui(ui, |ui| {
                for device in ptouch_core::device::supported_devices()
                    .iter()
                    .filter(|device| {
                        !device
                            .flags
                            .intersects(DeviceFlags::PLITE | DeviceFlags::UNSUP_RASTER)
                    })
                {
                    ui.selectable_value(
                        &mut state.tcp_pid,
                        device.pid,
                        format!("{} (0x{:04x})", device.name, device.pid),
                    );
                }
            });
        connect_tcp = ui.button("Connect TCP").clicked();
    });
    if let Some(candidate) = selected_network {
        state.tcp_address = candidate.address.to_string();
        if let Some(pid) = candidate.pid {
            state.tcp_pid = pid;
            connect_tcp = true;
        } else {
            state.printer_status = Some("Select the printer model, then connect".to_string());
        }
    }
    if connect_tcp {
        match state.tcp_address.parse::<std::net::SocketAddr>() {
            Ok(address) => {
                let target = PrinterTarget::Tcp {
                    address,
                    pid: state.tcp_pid,
                };
                state.printer_target = target.clone();
                state.printer_connected = false;
                state.printer_media_detected = false;
                state.printer_media_width_mm = None;
                state.manual_tcp_tape_width_mm = None;
                state.preview_uses_printer = false;
                state.update_tape_pixels();
                state.mark_dirty();
                state.printer_model = None;
                state.printer_status = Some("Connecting...".to_string());
                state.connecting = true;
                if let Some(tx) = &state.printer_cmd_tx {
                    let _ = tx.send(PrinterCommand::Poll(target));
                }
            }
            Err(error) => {
                state.printer_status = Some(format!("Invalid TCP address: {error}"));
            }
        }
    }

    let model_text = state.printer_model.as_deref().unwrap_or("Not connected");
    ui.label(format!("Model: {}", model_text));

    let status_text = state.printer_status.as_deref().unwrap_or("Scanning...");
    ui.label(format!("Status: {}", status_text));

    ui.add_space(4.0);
    if ui
        .add_enabled(
            !state.is_printer_busy() && state.printer_cmd_tx.is_some(),
            egui::Button::new("Refresh"),
        )
        .clicked()
        && let Some(ref tx) = state.printer_cmd_tx
    {
        state.connecting = true;
        let _ = tx.send(PrinterCommand::Poll(state.printer_target.clone()));
        let _ = tx.send(PrinterCommand::DiscoverBluetooth);
        let _ = tx.send(PrinterCommand::DiscoverNetwork);
        info!("Manual printer refresh requested");
    }
}

/// Tape width selection section.
fn show_tape_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Tape");
    ui.add_space(4.0);

    let tapes = tape::supported_tapes(state.target_dpi());
    let narrow_tcp_head = match state.printer_target {
        crate::state::PrinterTarget::Tcp { pid, .. } => {
            ptouch_core::device::find_device(ptouch_core::device::BROTHER_VENDOR_ID, pid)
                .is_some_and(|device| device.max_px <= 128)
        }
        _ => false,
    };
    let tcp_status_available = matches!(
        state.printer_target,
        crate::state::PrinterTarget::Tcp { .. }
    ) && state.printer_media_status_available;
    let detected_tcp_media = tcp_status_available && state.printer_media_width_mm.is_some();
    let manual_tcp_media = matches!(
        state.printer_target,
        crate::state::PrinterTarget::Tcp { .. }
    ) && state.printer_connected
        && !state.printer_media_status_available;
    let current_label = if manual_tcp_media && state.manual_tcp_tape_width_mm.is_none() {
        "Select loaded tape width".to_string()
    } else {
        format!("{} mm ({} px)", state.tape_width_mm, state.tape_width_px)
    };

    ui.add_enabled_ui(
        (!state.printer_target.is_bluetooth() || !state.printer_connected) && !tcp_status_available,
        |ui| {
            egui::ComboBox::from_label("Width")
                .selected_text(&current_label)
                .show_ui(ui, |ui| {
                    for t in tapes
                        .iter()
                        .filter(|t| !narrow_tcp_head || (t.width_mm != 21 && t.width_mm <= 24))
                    {
                        let label = format!("{} mm ({} px)", t.width_mm, t.pixels);
                        let changed = if manual_tcp_media {
                            ui.selectable_label(
                                state.manual_tcp_tape_width_mm == Some(t.width_mm),
                                &label,
                            )
                            .clicked()
                        } else {
                            ui.selectable_value(&mut state.tape_width_mm, t.width_mm, &label)
                                .changed()
                        };
                        if changed {
                            if manual_tcp_media {
                                state.manual_tcp_tape_width_mm = Some(t.width_mm);
                                state.tape_width_mm = t.width_mm;
                            }
                            state.update_tape_pixels();
                            state.mark_dirty();
                            info!("Tape changed: {} mm", t.width_mm);
                        }
                    }
                });
        },
    );
    if detected_tcp_media {
        ui.small("Tape width follows the detected cassette.");
    } else if tcp_status_available {
        ui.small("No tape detected. Load media and refresh.");
    } else if matches!(
        state.printer_target,
        crate::state::PrinterTarget::Tcp { .. }
    ) && state.printer_connected
    {
        ui.small("Tape status unavailable. Select the loaded width manually.");
    }

    egui::ComboBox::from_label("Design DPI")
        .selected_text(state.design_dpi.to_string())
        .show_ui(ui, |ui| {
            for dpi in [180, 360] {
                if ui.selectable_value(&mut state.design_dpi, dpi, dpi.to_string()).changed()
                    && !state.printer_connected
                {
                    state.preview_uses_printer = false;
                    state.update_tape_pixels();
                    state.mark_dirty();
                }
            }
        }).response.on_hover_text("Saved with the layout; used for offline preview and export. Connected printers use their own DPI.");
    let source = if state.printer_connected {
        "printer"
    } else if state.preview_uses_printer {
        "last printer"
    } else {
        "design"
    };
    ui.label(format!("Preview: {} DPI ({source})", state.target_dpi()));
    ui.small(format!("Printable height: {:.2} mm", state.printable_height_mm()))
        .on_hover_text("The printable area is narrower than the nominal tape. Layout Y coordinates start at its top edge.");
    if !state.printer_connected
        && state.preview_uses_printer
        && ui.button("Use design DPI").clicked()
    {
        state.preview_uses_printer = false;
        state.update_tape_pixels();
        state.mark_dirty();
    }
}

/// Print options: auto-cut toggle and quality selection.
fn show_print_options(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Print Options");
    ui.add_space(4.0);
    ui.add_enabled_ui(!state.printer_target.is_bluetooth(), |ui| {
        ui.horizontal(|ui| {
            let label = if state.auto_cut { "Auto cut" } else { "No cut" };
            ui.label(label);
            if ui
                .add(crate::widgets::toggle(&mut state.auto_cut))
                .changed()
            {
                state.calibration_enabled = false;
            }
        })
    });
    if state.printer_target.is_bluetooth() {
        ui.label("Manual cutter");
    }

    if state.printer_quality_modes {
        let quality_label = |q: PrintQuality| match q {
            PrintQuality::Standard => "Standard",
            PrintQuality::HighRes => "High resolution",
            PrintQuality::Draft => "Draft (high speed)",
        };
        egui::ComboBox::from_label("Quality")
            .selected_text(quality_label(state.print_quality))
            .show_ui(ui, |ui| {
                let qualities = if state.printer_draft_quality_modes {
                    vec![
                        PrintQuality::Standard,
                        PrintQuality::HighRes,
                        PrintQuality::Draft,
                    ]
                } else {
                    vec![PrintQuality::Standard, PrintQuality::HighRes]
                };
                for q in qualities {
                    if ui
                        .selectable_value(&mut state.print_quality, q, quality_label(q))
                        .changed()
                    {
                        state.calibration_enabled = false;
                    }
                }
            });
    }
    show_calibration(ui, state);
}

fn show_calibration(ui: &mut egui::Ui, state: &mut AppState) {
    ui.collapsing("Position calibration", |ui| {
        ui.add_enabled_ui(!state.is_printer_busy(), |ui| {
            ui.checkbox(&mut state.calibration_enabled, "Use custom correction");
            ui.small("Default keeps the printer's current behavior.");
            ui.horizontal(|ui| {
                ui.label("Offset:");
                ui.add(
                    crate::widgets::decimal_drag(&mut state.calibration.offset_mm)
                        .range(-100.0..=100.0)
                        .min_decimals(2)
                        .max_decimals(3)
                        .speed(0.1)
                        .suffix(" mm"),
                );
            });
            ui.small("Positive adds leading space. Negative removes blank space only.");
            ui.label("Saved profile:");
            egui::ComboBox::from_id_salt("calibration_profile")
                .selected_text(&state.calibration.name)
                .show_ui(ui, |ui| {
                    for profile in state.calibration_profiles.values() {
                        if ui
                            .selectable_label(state.calibration.name == profile.name, &profile.name)
                            .clicked()
                        {
                            state.calibration = profile.clone();
                            state.calibration_enabled = true;
                        }
                    }
                });
            ui.add(
                egui::TextEdit::singleline(&mut state.calibration.name)
                    .desired_width(ui.available_width())
                    .hint_text("Printer and cut mode"),
            );
            ui.small("Save a separate profile for each printer and cut mode.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Save profile").clicked() {
                    match state.calibration.validate() {
                        Ok(()) => {
                            state
                                .calibration_profiles
                                .insert(state.calibration.name.clone(), state.calibration.clone());
                            state.status_message = "Calibration profile saved".into();
                        }
                        Err(e) => state.status_message = e.to_string(),
                    }
                }
                if ui.button("Default").clicked() {
                    state.calibration_enabled = false;
                }
                if ui.button("Load file").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Calibration", &["toml"])
                        .pick_file()
                {
                    match std::fs::read_to_string(path)
                        .map_err(|e| e.to_string())
                        .and_then(|text| {
                            ptouch_render::calibration::PrintCalibration::from_toml(&text)
                                .map_err(|e| e.to_string())
                        }) {
                        Ok(profile) => {
                            state.calibration = profile;
                            state.calibration_enabled = true;
                        }
                        Err(e) => state.status_message = e,
                    }
                }
                if ui.button("Export profile").clicked() {
                    match state.calibration.to_toml() {
                        Ok(text) => {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Calibration", &["toml"])
                                .set_file_name("printer-calibration.toml")
                                .save_file()
                            {
                                match std::fs::write(path, text) {
                                    Ok(()) => {
                                        state.status_message = "Calibration profile exported".into()
                                    }
                                    Err(e) => state.status_message = e.to_string(),
                                }
                            }
                        }
                        Err(e) => state.status_message = e.to_string(),
                    }
                }
            });
            if ui
                .add_enabled(
                    state.preview_bitmap.is_some(),
                    egui::Button::new("Export corrected image"),
                )
                .clicked()
                && let Some(bitmap) = &state.preview_bitmap
            {
                match ptouch_render::calibration::apply_calibration(
                    bitmap,
                    state.target_dpi(),
                    state
                        .calibration_enabled
                        .then_some(state.calibration.offset_mm),
                ) {
                    Ok(corrected) => {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PNG", &["png"])
                            .save_file()
                        {
                            match corrected.save(&path) {
                                Ok(()) => state.status_message = "Corrected image exported".into(),
                                Err(e) => state.status_message = e.to_string(),
                            }
                        }
                    }
                    Err(e) => state.status_message = e.to_string(),
                }
            }
        });
    });
}

/// Whole-label mirror section: flip the entire composed label.
///
/// This applies once after all elements are composed and is separate from the
/// per-element flip on the properties panel.
fn show_mirror_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Mirror");
    ui.add_space(4.0);

    if ui
        .checkbox(&mut state.overall_flip_h, "Flip horizontal (left-right)")
        .changed()
    {
        state.mark_dirty();
        info!("Whole-label horizontal flip: {}", state.overall_flip_h);
    }
    if ui
        .checkbox(&mut state.overall_flip_v, "Flip vertical (top-bottom)")
        .changed()
    {
        state.mark_dirty();
        info!("Whole-label vertical flip: {}", state.overall_flip_v);
    }
}

/// Element list section with reorder and delete controls.
fn show_elements_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Elements");
    ui.small("Select an object. Drag rows to reorder.");
    ui.add_space(4.0);

    if state.elements.is_empty() {
        ui.label("(no elements)");
        return;
    }

    let mut action: Option<ElementAction> = None;
    let mut selection = None;
    let selected = state.selected_canvas_target();

    for (idx, element) in state.elements.iter().enumerate() {
        let object_target = CanvasTarget {
            element: idx,
            edit: CanvasEditTarget::Content,
        };
        let slot_target = CanvasTarget {
            element: idx,
            edit: CanvasEditTarget::Slot,
        };
        let has_slot = matches!(
            element,
            crate::state::LabelElement::Object(_)
                | crate::state::LabelElement::Text {
                    text_box: Some(_),
                    ..
                }
        );
        let flow = element.as_object().is_some_and(|o| {
            o.frame.position == ptouch_render::document::PositionMode::Relative
                || o.layout
                    .as_ref()
                    .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Flow)
        }) || !state.slot_followers(idx).is_empty();
        let label = format!(
            "{}. {} {}",
            idx + 1,
            if flow { "↔" } else { "•" },
            element.display_name()
        );
        let response = ui
            .horizontal(|ui| {
                let row_width = ui.available_width().max(20.0);

                ui.add_sized(
                    [row_width, ui.spacing().interact_size.y],
                    egui::Button::selectable(
                        selected.is_some_and(|t| t.element == idx),
                        (&label, egui::Atom::grow()),
                    )
                    .truncate()
                    .sense(egui::Sense::click_and_drag()),
                )
                .on_hover_text(format!(
                    "{label}\n{}",
                    if flow {
                        "In sequence: reordering changes physical placement."
                    } else {
                        "Free position: reordering preserves X/Y coordinates."
                    }
                ))
            })
            .inner;
        if response.clicked() || response.drag_started() {
            selection = Some(object_target);
        }
        response.context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                action = Some(ElementAction::Duplicate(idx));
                ui.close();
            }
            if idx > 0 {
                if state.can_swap_positions(idx, idx - 1)
                    && ui
                        .button("Swap positions with previous")
                        .on_hover_text(
                            "Exchange X/Y coordinates; each object keeps its size and content",
                        )
                        .clicked()
                {
                    action = Some(ElementAction::SwapPositions(idx));
                    ui.close();
                }
                if state.can_swap_content(idx, idx - 1)
                    && ui
                        .button("Swap content with previous")
                        .on_hover_text(
                            "Exchange content and its transforms while keeping layout allocations",
                        )
                        .clicked()
                {
                    action = Some(ElementAction::SwapContent(idx));
                    ui.close();
                }
            }
            if ui.button("Edit content").clicked() {
                selection = Some(object_target);
                ui.close();
            }
            if ui
                .add_enabled(has_slot, egui::Button::new("Adjust layout…"))
                .clicked()
            {
                selection = Some(slot_target);
                ui.close();
            }
        });
        response.dnd_set_drag_payload(ElementDrag(idx));
        if let Some(source) = response.dnd_hover_payload::<ElementDrag>() {
            let y = if source.0 < idx {
                response.rect.bottom()
            } else {
                response.rect.top()
            };
            ui.painter().line_segment(
                [
                    egui::pos2(response.rect.left(), y),
                    egui::pos2(response.rect.right(), y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(205, 115, 0)),
            );
        }
        if let Some(source) = response.dnd_release_payload::<ElementDrag>() {
            action = Some(ElementAction::MoveTo(source.0, idx));
        }
    }
    if let Some(target) = selection {
        state.select_canvas_target(Some(target));
    }

    ui.add_space(4.0);

    ui.horizontal_wrapped(|ui| {
        let has_selection = state.selected_element.is_some();
        if ui
            .add_enabled(has_selection, egui::Button::new("Duplicate"))
            .on_hover_text(
                "Copy with its settings: after the source in a sequence, or at the end for free positioning (Ctrl+D / Cmd+D)",
            )
            .clicked()
            && let Some(idx) = state.selected_element
        {
            action = Some(ElementAction::Duplicate(idx));
        }

        if ui
            .add_enabled(
                has_selection && state.selected_element.unwrap_or(0) > 0,
                egui::Button::new("Up"),
            )
            .on_hover_text("Reorder a sequence; free objects keep their X/Y coordinates")
            .clicked()
            && let Some(idx) = state.selected_element
        {
            action = Some(ElementAction::MoveUp(idx));
        }

        if ui
            .add_enabled(
                has_selection
                    && state
                        .selected_element
                        .map(|i| i + 1 < state.elements.len())
                        .unwrap_or(false),
                egui::Button::new("Down"),
            )
            .on_hover_text("Reorder a sequence; free objects keep their X/Y coordinates")
            .clicked()
            && let Some(idx) = state.selected_element
        {
            action = Some(ElementAction::MoveDown(idx));
        }

        if ui
            .add_enabled(has_selection, egui::Button::new("Delete"))
            .clicked()
            && let Some(idx) = state.selected_element
        {
            action = Some(ElementAction::Delete(idx));
        }
    });

    // Apply deferred action
    if let Some(act) = action {
        match act {
            ElementAction::SwapPositions(index) => {
                state.swap_positions(index, index - 1);
            }
            ElementAction::SwapContent(index) => {
                state.swap_content(index, index - 1);
            }
            ElementAction::Duplicate(idx) => {
                state.duplicate_element(idx);
            }
            ElementAction::MoveUp(idx) => {
                if idx > 0 {
                    move_element(state, idx, idx - 1);
                }
            }
            ElementAction::MoveDown(idx) => {
                if idx + 1 < state.elements.len() {
                    move_element(state, idx, idx + 1);
                }
            }
            ElementAction::MoveTo(from, to) => move_element(state, from, to),
            ElementAction::Delete(idx) => {
                state.delete_element(idx);
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ElementDrag(usize);

fn move_element(state: &mut AppState, from: usize, to: usize) {
    state.reorder_object(from, to);
}

/// Deferred actions for element list manipulation.
enum ElementAction {
    SwapPositions(usize),
    SwapContent(usize),
    Duplicate(usize),
    MoveUp(usize),
    MoveDown(usize),
    MoveTo(usize, usize),
    Delete(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::document::{LabelElement, LabelObject, PositionMode, TextBox};

    fn list_frame(
        context: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 500.0),
                )),
                events,
                ..Default::default()
            },
            |ui| show_elements_section(ui, state),
        );
        output.textures_delta.clear();
        output
    }

    fn key_events(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .into_iter()
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            })
            .collect()
    }

    fn pointer_event(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        }
    }

    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) if shape.galley.text().starts_with(text) => {
                    Some(shape.pos + shape.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing text: {text}"))
    }

    #[test]
    fn row_selects_one_object_and_layout_is_an_explicit_context_action() {
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
        let context = egui::Context::default();
        list_frame(&context, &mut state, vec![]);
        list_frame(&context, &mut state, key_events(egui::Key::Tab));
        list_frame(&context, &mut state, key_events(egui::Key::Tab));
        let output = list_frame(&context, &mut state, key_events(egui::Key::Enter));
        assert_eq!(
            state.selected_canvas_target(),
            Some(CanvasTarget {
                element: 0,
                edit: CanvasEditTarget::Content,
            })
        );
        let row = text_position(&output, "1. ");
        list_frame(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(row),
                pointer_event(row, egui::PointerButton::Primary, true),
            ],
        );
        list_frame(
            &context,
            &mut state,
            vec![pointer_event(row, egui::PointerButton::Primary, false)],
        );
        assert_eq!(
            state.selected_canvas_target().unwrap().edit,
            CanvasEditTarget::Content
        );
        list_frame(
            &context,
            &mut state,
            vec![pointer_event(row, egui::PointerButton::Secondary, true)],
        );
        list_frame(
            &context,
            &mut state,
            vec![pointer_event(row, egui::PointerButton::Secondary, false)],
        );
        let menu = list_frame(&context, &mut state, vec![]);
        let slot = text_position(&menu, "Adjust layout…");
        list_frame(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(slot),
                pointer_event(slot, egui::PointerButton::Primary, true),
            ],
        );
        list_frame(
            &context,
            &mut state,
            vec![pointer_event(slot, egui::PointerButton::Primary, false)],
        );
        assert_eq!(
            state.selected_canvas_target().unwrap().edit,
            CanvasEditTarget::Slot
        );
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        assert_eq!(state.document_version, 3);
        assert!(!state.needs_rerender);
    }

    #[test]
    fn reordering_keeps_chain_anchor_and_rejects_crossing_free_objects() {
        let mut state = AppState {
            elements: vec![
                LabelElement::Object(LabelObject::text(
                    "A",
                    TextBox {
                        x_mm: 5.0,
                        ..TextBox::default()
                    },
                )),
                LabelElement::Object(LabelObject::text(
                    "B",
                    TextBox {
                        x_mm: 30.0,
                        ..TextBox::default()
                    },
                )),
                LabelElement::Object(LabelObject::text(
                    "C",
                    TextBox {
                        position: PositionMode::Relative,
                        gap_mm: 3.0,
                        ..TextBox::default()
                    },
                )),
            ],
            ..AppState::default()
        };
        assert_eq!(state.element_rect_mm(2).unwrap().0, 43.0);
        state.select_canvas_target(Some(CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Slot,
        }));
        let before = state.document();
        move_element(&mut state, 0, 1);
        assert_eq!(state.document(), before);
        move_element(&mut state, 2, 1);
        assert_eq!(state.selected_element, Some(1));
        assert_eq!(state.canvas_edit_target, CanvasEditTarget::Slot);
        assert_eq!(state.element_rect_mm(0).unwrap().0, 5.0);
        assert_eq!(state.element_rect_mm(1).unwrap().0, 30.0);
        assert_eq!(state.element_rect_mm(2).unwrap().0, 43.0);
        assert!(
            matches!(&state.elements[1].as_object().unwrap().content, ptouch_render::document::ObjectContent::Text { content, .. } if content == "C")
        );
    }
}
