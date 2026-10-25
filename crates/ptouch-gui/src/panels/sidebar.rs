// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Left sidebar panel: printer info, tape selection, and element list.

use log::info;

use ptouch_core::protocol::PrintQuality;
use ptouch_core::tape;

use crate::state::{AppState, CanvasEditTarget, CanvasTarget, PrinterCommand};

/// Render the left sidebar.
pub fn show_sidebar(ui: &mut egui::Ui, state: &mut AppState) {
    let rows = if state.layers_view { 3.0 } else { 2.0 };
    let footer_height = rows * (ui.spacing().interact_size.y + ui.spacing().item_spacing.y) + 4.0;
    egui::Panel::bottom("element_actions")
        .exact_size(footer_height)
        .show(ui, |ui| {
            show_element_actions(ui, state);
        });
    let settings_height = (ui.available_height() * 0.4).clamp(80.0, 260.0);
    egui::CollapsingHeader::new("Printer and tape")
        .default_open(true)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("printer_settings")
                .max_height(settings_height)
                .show(ui, |ui| {
                    show_printer_section(ui, state);
                    ui.separator();
                    show_tape_section(ui, state);
                    ui.separator();
                    show_print_options(ui, state);
                    ui.separator();
                    show_mirror_section(ui, state);
                });
        });
    ui.separator();
    show_elements_section(ui, state);
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
        state.printer_status = Some("Connecting...".to_string());
        state.connecting = true;
        state.auto_cut = !state.printer_target.is_bluetooth();
        if let Some(ref tx) = state.printer_cmd_tx {
            let _ = tx.send(PrinterCommand::Poll(state.printer_target.clone()));
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
        info!("Manual printer refresh requested");
    }
}

/// Tape width selection section.
fn show_tape_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Tape");
    ui.add_space(4.0);

    let tapes = tape::supported_tapes(state.target_dpi());
    let current_label = format!("{} mm ({} px)", state.tape_width_mm, state.tape_width_px);

    ui.add_enabled_ui(
        !state.printer_target.is_bluetooth() || !state.printer_connected,
        |ui| {
            egui::ComboBox::from_label("Width")
                .selected_text(&current_label)
                .show_ui(ui, |ui| {
                    for t in tapes {
                        let label = format!("{} mm ({} px)", t.width_mm, t.pixels);
                        if ui
                            .selectable_value(&mut state.tape_width_mm, t.width_mm, &label)
                            .clicked()
                        {
                            state.update_tape_pixels();
                            state.mark_dirty();
                            info!("Tape changed: {} mm", t.width_mm);
                        }
                    }
                });
        },
    );

    egui::ComboBox::from_label("Design DPI")
        .selected_text(state.design_dpi.to_string())
        .show_ui(ui, |ui| {
            let mut selected_dpi = state.design_dpi;
            for dpi in [180, 360] {
                if ui.selectable_value(&mut selected_dpi, dpi, dpi.to_string()).changed() {
                    state.set_design_dpi(selected_dpi);
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
                for q in [
                    PrintQuality::Standard,
                    PrintQuality::HighRes,
                    PrintQuality::Draft,
                ] {
                    if ui
                        .selectable_value(&mut state.print_quality, q, quality_label(q))
                        .changed()
                    {
                        state.calibration_enabled = false;
                    }
                }
            });
    }
    crate::output_tools::show_controls(ui, state);
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
                    crate::output_tools::preview_ready(state),
                    egui::Button::new("Export corrected image"),
                )
                .clicked()
            {
                crate::output_tools::export_corrected(state);
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

/// Element rows scroll independently of settings and actions.
fn show_elements_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Elements");
    ui.horizontal(|ui| {
        for (layers, label) in [(false, "Layout order"), (true, "Layers")] {
            if ui
                .selectable_value(&mut state.layers_view, layers, label)
                .clicked()
            {
                crate::editor_navigation::set_focus(
                    ui.ctx(),
                    crate::editor_navigation::EditorFocus::Elements,
                );
            }
        }
    });
    let scroll = egui::ScrollArea::vertical()
        .id_salt((
            if state.layers_view {
                "layer_rows"
            } else {
                "element_rows"
            },
            state.file_view_generation,
        ))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.layers_view {
                super::scene_tools::show_layers(ui, state);
            } else {
                ui.small("Drag rows to change their order.");
                if state.elements.is_empty() {
                    ui.label("(no elements)");
                } else {
                    show_element_rows(ui, state);
                }
            }
        });
    if ui.input(|input| input.pointer.primary_pressed())
        && let Some(pointer) = ui.input(|input| input.pointer.interact_pos())
        && scroll.inner_rect.contains(pointer)
        && ui.ctx().layer_id_at(pointer) == Some(ui.layer_id())
    {
        crate::editor_navigation::set_focus(
            ui.ctx(),
            crate::editor_navigation::EditorFocus::Elements,
        );
    }
    crate::editor_navigation::scroll_drag_near_edge(ui, &scroll);
}

fn show_element_rows(ui: &mut egui::Ui, state: &mut AppState) {
    let mut action: Option<ElementAction> = None;
    let mut selection = None;
    let order = state.layout_order();
    crate::editor_navigation::list_keyboard(ui, state, &order);
    let selected_indices = state.selected_indices();
    for (row, idx) in order.iter().copied().enumerate() {
        let element = &state.elements[idx];
        let object_target = CanvasTarget {
            element: idx,
            edit: CanvasEditTarget::Content,
        };
        let slot_target = CanvasTarget {
            element: idx,
            edit: CanvasEditTarget::Slot,
        };
        let attached = element.as_object().is_some_and(|object| {
            object
                .scene
                .as_ref()
                .is_some_and(|scene| scene.anchor.is_some())
        });
        if !attached
            && let Some(object) = element.as_object()
            && object.sequence.is_some()
            && object.frame.position == ptouch_render::document::PositionMode::Absolute
        {
            let x = object.frame.x_mm;
            ui.horizontal_wrapped(|ui| {
                ui.small(format!("Sequence · X {x:.2} mm"))
                    .on_hover_text("Fixed sequence start. Up/Down stays in this sequence; dragging into another sequence moves the whole object.");
                if state.previous_sequence_end(idx).is_some()
                    && ui.small_button("Connect")
                        .on_hover_text("Connect to the previous sequence without moving anything. Then adjust Gap before for continuous spacing.")
                        .clicked()
                {
                    action = Some(ElementAction::Connect(idx));
                }
            });
        }
        let has_slot = !attached
            && matches!(
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
            "{}. {} {}{}",
            row + 1,
            if flow { "↔" } else { "•" },
            element.display_name(),
            if element
                .as_object()
                .is_some_and(|o| o.scene.as_ref().is_some_and(|s| s.anchor.is_some()))
            {
                " ↗"
            } else {
                ""
            }
        );
        let response = ui
            .horizontal(|ui| {
                if attached {
                    ui.add_space(12.0);
                }
                let row_width = ui.available_width().max(20.0);

                ui.add_sized(
                    [row_width, ui.spacing().interact_size.y],
                    egui::Button::selectable(
                        selected_indices.contains(&idx),
                        (&label, egui::Atom::grow()),
                    )
                    .truncate()
                    .sense(egui::Sense::click_and_drag()),
                )
                .on_hover_text(format!(
                    "{label}\n{}",
                    if attached {
                        "Attached: moves with its host. Use Layers to change drawing order."
                    } else if flow {
                        "In sequence: reordering changes physical placement."
                    } else {
                        "Free position: reordering preserves X/Y coordinates."
                    }
                ))
            })
            .inner;
        crate::editor_navigation::reveal_selected_row(ui, &response, state, idx);
        if response.clicked() || response.drag_started() {
            let modifiers = ui.input(|input| input.modifiers);
            if !response.drag_started()
                || (!(modifiers.shift || modifiers.command || modifiers.ctrl)
                    && (selected_indices.len() <= 1 || !selected_indices.contains(&idx)))
            {
                state.select_list_item(idx, modifiers, &order);
                crate::editor_navigation::set_focus(
                    ui.ctx(),
                    crate::editor_navigation::EditorFocus::Elements,
                );
                crate::editor_navigation::request_canvas_reveal(ui.ctx(), idx);
            }
        }
        response.context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                action = Some(ElementAction::Duplicate(idx));
                ui.close();
            }
            if let Some(previous) = state.previous_layout_index(idx).filter(|_| !attached) {
                if state.can_swap_positions(idx, previous)
                    && ui
                        .button("Swap positions with previous")
                        .on_hover_text(
                            "Exchange X/Y coordinates; each object keeps its size and content",
                        )
                        .clicked()
                {
                    action = Some(ElementAction::SwapPositions(idx, previous));
                    ui.close();
                }
                if state.can_swap_content(idx, previous)
                    && ui
                        .button("Swap content with previous")
                        .on_hover_text(
                            "Exchange content and its transforms while keeping layout allocations",
                        )
                        .clicked()
                {
                    action = Some(ElementAction::SwapContent(idx, previous));
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
        if !attached
            && selected_indices.len() <= 1
            && !ui.input(|input| {
                input.modifiers.shift || input.modifiers.command || input.modifiers.ctrl
            })
        {
            response.dnd_set_drag_payload(ElementDrag(idx));
        }
        if let Some(source) = response.dnd_hover_payload::<ElementDrag>()
            && state.layout_host(idx) != Some(source.0)
        {
            let y = if source.0 < idx {
                response.rect.bottom()
            } else {
                response.rect.top()
            };
            let hint = match (
                state.flow_chain(source.0),
                state.layout_host(idx).and_then(|i| state.flow_chain(i)),
            ) {
                (Some(from), Some(to)) if from != to => "Move into fixed sequence · gap retained",
                (Some(_), _) => "Reorder sequence",
                _ => "List order · keep position",
            };
            let color = egui::Color32::from_rgb(235, 160, 60);
            let text =
                ui.painter()
                    .layout_no_wrap(hint.into(), egui::FontId::proportional(11.0), color);
            let position = egui::pos2(
                response.rect.right() - text.size().x - 2.0,
                y - text.size().y - 2.0,
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_size(position, text.size() + egui::vec2(2.0, 2.0)),
                2.0,
                ui.visuals().panel_fill,
            );
            ui.painter().galley(position, text, color);
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
        state.set_selected_indices(&[target.element], Some(target.element));
        state.select_canvas_target(Some(target));
        crate::editor_navigation::set_focus(
            ui.ctx(),
            crate::editor_navigation::EditorFocus::Elements,
        );
        crate::editor_navigation::request_canvas_reveal(ui.ctx(), target.element);
    }

    if let Some(action) = action {
        apply_element_action(state, action);
    }
}

fn show_element_actions(ui: &mut egui::Ui, state: &mut AppState) {
    let mut action = None;
    let selected = state.selected_element;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Duplicate"))
            .clicked()
            && let Some(index) = selected
        {
            action = Some(ElementAction::Duplicate(index));
        }
        if ui
            .add_enabled(selected.is_some(), egui::Button::new("Delete"))
            .on_hover_text("Delete selected objects and their attached objects.")
            .clicked()
            && let Some(index) = selected
        {
            action = Some(ElementAction::Delete(index));
        }
    });
    if state.layers_view {
        super::scene_tools::show_layer_actions(ui, state);
    } else {
        ui.horizontal(|ui| {
            for (label, forward) in [("Up", false), ("Down", true)] {
                let next = selected.and_then(|index| state.layout_neighbor(index, forward));
                if ui
                    .add_enabled(
                        next.is_some() && state.selected_indices().len() == 1,
                        egui::Button::new(label),
                    )
                    .on_hover_text("Change the order within this sequence")
                    .clicked()
                    && let (Some(index), Some(next)) = (selected, next)
                {
                    action = Some(ElementAction::MoveTo(index, next));
                }
            }
        });
    }
    if let Some(action) = action {
        apply_element_action(state, action);
    }
}

fn apply_element_action(state: &mut AppState, act: ElementAction) {
    match act {
        ElementAction::SwapPositions(index, previous) => {
            state.swap_positions(index, previous);
        }
        ElementAction::SwapContent(index, previous) => {
            state.swap_content(index, previous);
        }
        ElementAction::Connect(index) => {
            state.connect_sequence(index);
        }
        ElementAction::Duplicate(idx) => {
            if !state.selected_indices().contains(&idx) {
                state.set_selected_indices(&[idx], Some(idx));
            }
            state.copy_selection();
        }
        ElementAction::MoveTo(from, to) => move_element(state, from, to),
        ElementAction::Delete(idx) => {
            if !state.selected_indices().contains(&idx) {
                state.set_selected_indices(&[idx], Some(idx));
            }
            state.delete_selection();
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
    SwapPositions(usize, usize),
    SwapContent(usize, usize),
    Connect(usize),
    Duplicate(usize),
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
        let output = list_frame(&context, &mut state, vec![]);
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
            state.selected_canvas_target(),
            Some(CanvasTarget {
                element: 0,
                edit: CanvasEditTarget::Content,
            })
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
    fn reordering_keeps_chain_anchor_with_interleaved_free_objects() {
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
        move_element(&mut state, 0, 1);
        assert_eq!(state.element_rect_mm(0).unwrap().0, 30.0);
        assert_eq!(state.element_rect_mm(1).unwrap().0, 5.0);
        assert_eq!(state.element_rect_mm(2).unwrap().0, 43.0);
        assert_eq!(state.flow_chain(0), Some(vec![0, 2]));
        move_element(&mut state, 1, 0);
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
