// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Right properties panel for editing selected element attributes.

use log::{error, info};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use ptouch_render::document::{
    ContentBox, LabelObject, ObjectContent, PositionMode, TextBox, VerticalAlign,
};
use ptouch_render::image_loader;
use ptouch_render::text::TextAlign;

use crate::alignment::ContentAlignment;
use crate::positioning::Positioning;
use crate::state::{AppState, CanvasEditTarget, CanvasTarget, LabelElement};

/// Render the right-side properties panel.
pub fn show_properties(ui: &mut egui::Ui, state: &mut AppState) {
    super::repeat_tools::buttons(ui, state);
    if state.selected_indices().len() > 1 {
        ui.push_id("multiple_objects", |ui| {
            super::selection_tools::show(ui, state)
        });
        return;
    }
    // Loaded history or a completed relation edit can retain a previous layout
    // target. Apply the shared attachment selection rule before widget IDs.
    state.select_canvas_target(state.selected_canvas_target());
    ui.push_id(
        (
            state.selection_revision,
            state.selected_element,
            state.canvas_edit_target == CanvasEditTarget::Slot,
        ),
        |ui| {
            show_selected_properties(ui, state);
        },
    );
}

fn show_selected_properties(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Object");
    ui.add_space(4.0);

    let selected = match state.selected_element {
        Some(idx) if idx < state.elements.len() => idx,
        _ => {
            ui.label("Select an element to edit properties");
            return;
        }
    };
    if super::scene_tools::show_attachment(ui, state) {
        return;
    }
    if let Some(frame) = state
        .position_frames()
        .get(selected)
        .and_then(Option::as_ref)
    {
        ui.label("Position");
        egui::Grid::new("resolved_position")
            .num_columns(2)
            .show(ui, |ui| {
                for (name, value) in [
                    ("X", frame.x_mm),
                    ("Y", frame.y_mm),
                    ("Right edge", frame.x_mm + frame.width_mm),
                ] {
                    ui.label(name);
                    ui.label(format!("{value:.3} mm"));
                    ui.end_row();
                }
            });
        ui.small("Positions start at the upper left corner of the printable area.");
        ui.separator();
    }

    // We need to clone the element to avoid borrow issues, then write back.
    let scene_font_before = state.uses_scene().then(|| state.font_name.clone());
    let mut element = state.elements[selected].clone();
    let mut changed = false;
    let mut alignment = None;
    let mut positioning = None;

    match &mut element {
        LabelElement::Object(object) => {
            changed |= show_object_properties(ui, object, state, &mut alignment, &mut positioning);
        }
        LabelElement::Text {
            content,
            font_size,
            align,
            rotation,
            flip_h,
            flip_v,
            text_box,
        } => {
            changed |= show_text_properties(
                ui,
                TextProps {
                    content,
                    font_size,
                    align,
                    rotation,
                    flip_h,
                    flip_v,
                    text_box,
                },
                state,
            );
        }
        LabelElement::Image {
            path,
            image_data,
            bitmap,
            rotation,
            target_height,
            flip_h,
            flip_v,
        } => {
            changed |= show_image_properties(
                ui,
                ImageProps {
                    path,
                    image_data,
                    bitmap,
                    rotation,
                    target_height,
                    flip_h,
                    flip_v,
                },
                state,
            );
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

    // The global font transaction resolves every host and attached child. Give
    // it this frame rather than writing stale local geometry over its result.
    if scene_font_before.is_some_and(|font| font != state.font_name) {
        return;
    }
    if changed || alignment.is_some() {
        if let (Some(before), Some(candidate)) = (
            state.elements[selected].as_object(),
            element.as_object_mut(),
        ) {
            let geometric = candidate.content_box != before.content_box
                || candidate.frame != before.frame
                || candidate.layout != before.layout
                || candidate.rotation != before.rotation
                || candidate.flip_h != before.flip_h
                || candidate.flip_v != before.flip_v;
            let became_free = candidate
                .layout
                .as_ref()
                .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Free)
                && before
                    .layout
                    .as_ref()
                    .is_none_or(|p| p.placement != ptouch_render::object_layout::Placement::Free);
            if geometric && !became_free {
                match state.constrain_transform(selected, before, candidate) {
                    Err(message) => {
                        state.status_message = message.into();
                        return;
                    }
                    Ok(clamped) => state.transform_feedback(
                        clamped,
                        candidate.layout.as_ref().is_some_and(|p| {
                            p.placement == ptouch_render::object_layout::Placement::Free
                        }),
                    ),
                }
            }
        }
        if let (Some(alignment), Some(candidate)) = (alignment, element.as_object_mut())
            && let Err(message) = state.align_candidate(selected, candidate, alignment)
        {
            state.status_message = message;
            return;
        }
        if alignment.is_some() && element == state.elements[selected] {
            return;
        }
        if element
            .as_object()
            .is_some_and(|object| object.content_box.is_some())
        {
            state.document_version = state.document_version.max(4);
        }
        if let LabelElement::Object(object) = element {
            if let Err(message) = state.apply_object_edit(selected, object) {
                state.status_message = message;
                return;
            }
        } else {
            state.elements[selected] = element;
            state.mark_dirty();
        }
    }
    if let Some(choice) = positioning
        && let Err(message) = state.set_positioning(selected, choice)
    {
        state.status_message = message;
    }
}

/// Mutable references to a text element's editable fields.
struct TextProps<'a> {
    content: &'a mut String,
    font_size: &'a mut Option<f32>,
    align: &'a mut TextAlign,
    rotation: &'a mut f32,
    flip_h: &'a mut bool,
    flip_v: &'a mut bool,
    text_box: &'a mut Option<TextBox>,
}

/// Mutable references to an image element's editable fields.
struct ImageProps<'a> {
    path: &'a mut Option<PathBuf>,
    image_data: &'a mut Vec<u8>,
    bitmap: &'a mut Option<ptouch_render::bitmap::LabelBitmap>,
    rotation: &'a mut f32,
    target_height: &'a mut Option<u32>,
    flip_h: &'a mut bool,
    flip_v: &'a mut bool,
}

/// Two checkboxes for per-element horizontal/vertical mirroring. Returns true if
/// either changed. Shared by the text and image property panels.
fn show_flip_controls(ui: &mut egui::Ui, flip_h: &mut bool, flip_v: &mut bool) -> bool {
    let mut changed = false;
    ui.label("Mirror:");
    if ui
        .checkbox(flip_h, "Flip horizontal (left-right)")
        .changed()
    {
        changed = true;
    }
    if ui.checkbox(flip_v, "Flip vertical (top-bottom)").changed() {
        changed = true;
    }
    changed
}

/// Show properties for a text element. Returns true if any value changed.
fn show_text_properties(ui: &mut egui::Ui, props: TextProps, state: &mut AppState) -> bool {
    let TextProps {
        content,
        font_size,
        align,
        rotation,
        flip_h,
        flip_v,
        text_box,
    } = props;
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

    let mut fixed_box = text_box.is_some();
    if ui.checkbox(&mut fixed_box, "Fixed text box (mm)").changed() {
        *text_box = fixed_box.then(|| {
            let x_mm = state
                .preview_bounds
                .iter()
                .find(|bound| Some(bound.element_index) == state.selected_element)
                .map_or(0.0, |bound| {
                    let x = if state.overall_flip_h {
                        state.preview_bitmap.as_ref().map_or(bound.x, |bitmap| {
                            bitmap.width().saturating_sub(bound.x + bound.width)
                        })
                    } else {
                        bound.x
                    };
                    f64::from(x) * 25.4 / f64::from(state.target_dpi())
                });
            TextBox {
                x_mm,
                height_mm: 8.0_f64
                    .min(f64::from(state.tape_width_px) * 25.4 / f64::from(state.target_dpi())),
                ..TextBox::default()
            }
        });
        if fixed_box {
            state.document_version = state.document_version.max(2);
        }
        if let Some(size) = font_size {
            *size *= if fixed_box {
                72.0 / f32::from(state.target_dpi())
            } else {
                f32::from(state.target_dpi()) / 72.0
            };
        }
        changed = true;
    }
    if let Some(bounds) = text_box {
        changed |= show_text_box(ui, bounds, state, true, None);
    } else {
        ui.small("Natural width; follows the previous element.");
    }
    ui.separator();

    changed |= show_font_properties(ui, font_size, align, text_box.is_some(), state);
    ui.add_space(4.0);

    // Rotation
    ui.label("Rotation:");
    ui.horizontal(|ui| {
        if ui
            .add(
                crate::widgets::decimal_drag(rotation)
                    .speed(1.0)
                    .clamp_existing_to_range(false)
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
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Ok(val) = state.rotation_input.parse::<f32>()
        {
            let clamped = val.clamp(-360.0, 360.0);
            *rotation = clamped;
            state.rotation_input = format!("{}", clamped);
            changed = true;
        }
    });

    ui.add_space(4.0);
    changed |= show_flip_controls(ui, flip_h, flip_v);

    changed
}

/// Text appearance is shared by legacy text and free-layout text. The font
/// family and margin belong to the document, while size/alignment are local.
fn show_font_properties(
    ui: &mut egui::Ui,
    font_size: &mut Option<f32>,
    align: &mut TextAlign,
    points: bool,
    state: &mut AppState,
) -> bool {
    let mut changed = false;
    ui.label("Default font (label):");
    ui.add(
        egui::TextEdit::singleline(&mut state.font_search)
            .desired_width(f32::INFINITY)
            .hint_text("Search fonts..."),
    );
    let query = state.font_search.to_lowercase();
    let mut selected_font = None;
    egui::ComboBox::from_id_salt("font_selector")
        .selected_text(&state.font_name)
        .width(150.0)
        .height(300.0)
        .show_ui(ui, |ui| {
            for font in &state.available_fonts {
                if !query.is_empty() && !font.to_lowercase().contains(&query) {
                    continue;
                }
                if ui
                    .selectable_label(state.font_name == *font, font)
                    .clicked()
                {
                    selected_font = Some(font.clone());
                }
            }
        });
    if let Some(font) = selected_font {
        if state.uses_scene() {
            match state.apply_scene_font(font) {
                Ok(()) => state.font_search.clear(),
                Err(error) => state.status_message = error,
            }
        } else {
            state.font_name = font;
            state.font_search.clear();
            changed = true;
        }
    }
    let mut auto = font_size.is_none();
    if ui.checkbox(&mut auto, "Auto font size").changed() {
        *font_size = (!auto).then_some(if points { 12.0 } else { 24.0 });
        changed = true;
    }
    if let Some(size) = font_size {
        ui.horizontal(|ui| {
            ui.label(if points { "Size (pt):" } else { "Size (px):" });
            changed |= ui
                .add(
                    crate::widgets::decimal_drag(size)
                        .speed(0.5)
                        .clamp_existing_to_range(false)
                        .range(1.0..=200.0),
                )
                .changed();
        });
    }
    ui.horizontal(|ui| {
        ui.label("Alignment:");
        for (value, label) in [
            (TextAlign::Left, "Left"),
            (TextAlign::Center, "Center"),
            (TextAlign::Right, "Right"),
        ] {
            changed |= ui.selectable_value(align, value, label).changed();
        }
    });
    ui.horizontal(|ui| {
        ui.label("Font margin (px):");
        changed |= ui
            .add(
                crate::widgets::decimal_drag(&mut state.font_margin)
                    .clamp_existing_to_range(false)
                    .range(0..=50),
            )
            .changed();
    });
    changed
}

/// Change the positioning mode without moving the frame. Relative layouts
/// intentionally use nonnegative gaps; overlapping frames remain absolute.
fn switch_position_mode(
    frame: &mut TextBox,
    mode: PositionMode,
    previous_right_mm: f64,
) -> Result<bool, &'static str> {
    if frame.position == mode {
        return Ok(false);
    }
    let x = match frame.position {
        PositionMode::Absolute => frame.x_mm,
        PositionMode::Relative => previous_right_mm + frame.gap_mm,
    };
    match mode {
        PositionMode::Absolute => frame.x_mm = x,
        PositionMode::Relative => {
            let gap = x - previous_right_mm;
            if gap < -1e-9 {
                return Err(
                    "This object overlaps or precedes the previous object. Keep Absolute positioning or move it after the previous right edge.",
                );
            }
            frame.gap_mm = gap.max(0.0);
        }
    }
    frame.position = mode;
    Ok(true)
}

fn show_text_box(
    ui: &mut egui::Ui,
    bounds: &mut TextBox,
    state: &mut AppState,
    show_alignment: bool,
    mut positioning: Option<&mut Option<Positioning>>,
) -> bool {
    let mut changed = false;
    let selected = state.selected_element.unwrap_or(0);
    let linked_previous = state
        .elements
        .get(selected)
        .and_then(|e| e.as_object())
        .filter(|o| o.sequence.is_some() && o.frame.position == PositionMode::Relative)
        .and_then(|_| state.flow_chain(selected))
        .and_then(|chain| chain.into_iter().take_while(|i| *i < selected).last());
    let previous_right = linked_previous
        .and_then(|i| state.element_rect_mm(i))
        .or_else(|| {
            (0..selected)
                .rev()
                .filter(|i| state.layout_host(*i) == Some(*i))
                .find_map(|i| state.element_rect_mm(i))
        })
        .map_or(0.0, |(x, _, w, _)| x + w);
    ui.horizontal(|ui| {
        for (mode, label) in [
            (PositionMode::Absolute, "Absolute"),
            (PositionMode::Relative, "After previous"),
        ] {
            if ui
                .selectable_label(bounds.position == mode, label)
                .clicked()
            {
                if let Some(positioning) = positioning.as_mut() {
                    **positioning = Some(match mode {
                        PositionMode::Absolute => Positioning::Absolute,
                        PositionMode::Relative => Positioning::AfterPrevious,
                    });
                    continue;
                }
                match switch_position_mode(bounds, mode, previous_right) {
                    Ok(did_change) => changed |= did_change,
                    Err(message) => state.status_message = message.into(),
                }
            }
        }
    });
    let minimum = 25.4 / f64::from(state.target_dpi());
    let printable_height = state.printable_height_mm();
    let mut height_changed = false;
    egui::Grid::new("object_frame_dimensions")
        .num_columns(2)
        .show(ui, |ui| {
            for (label, value, maximum) in [
                ("Allocation width", &mut bounds.width_mm, 10_000.0),
                ("Height", &mut bounds.height_mm, printable_height),
            ] {
                ui.label(label);
                let requested = std::cell::Cell::new(None);
                let edited = ui
                    .add(
                        crate::widgets::decimal_drag(value)
                            .custom_parser(|text| {
                                let parsed = crate::widgets::parse_decimal(text);
                                requested.set(parsed);
                                parsed
                            })
                            .speed(0.1)
                            .clamp_existing_to_range(false)
                            .range(minimum..=maximum)
                            .suffix(" mm"),
                    )
                    .changed();
                if label == "Height" && requested.get().is_some_and(|height| height > maximum) {
                    state.status_message = format!(
                        "Height limited to the printable area: {maximum:.3} mm on {} mm tape.",
                        state.tape_width_mm
                    );
                }
                changed |= edited;
                height_changed |= label == "Height" && edited;
                ui.end_row();
            }
            let (label, value) = match bounds.position {
                PositionMode::Absolute => ("X", &mut bounds.x_mm),
                PositionMode::Relative => ("Gap before", &mut bounds.gap_mm),
            };
            ui.label(label);
            changed |= ui
                .add(
                    crate::widgets::decimal_drag(value)
                        .speed(0.1)
                        .clamp_existing_to_range(false)
                        .range(0.0..=10_000.0)
                        .suffix(" mm"),
                )
                .changed();
            ui.end_row();
            ui.label("Y");
            changed |= ui
                .add(
                    crate::widgets::decimal_drag(&mut bounds.y_mm)
                        .speed(0.1)
                        .clamp_existing_to_range(false)
                        .range(0.0..=(printable_height - bounds.height_mm).max(0.0))
                        .suffix(" mm"),
                )
                .changed();
            ui.end_row();
        });
    if height_changed {
        bounds.y_mm = bounds
            .y_mm
            .min((printable_height - bounds.height_mm).max(0.0));
    }
    if show_alignment {
        changed |= show_vertical_alignment(ui, &mut bounds.vertical_align);
    }
    ui.small("Origin: printable top-left, not the cut edge. Rotation keeps this frame fixed.");
    if bounds.position == PositionMode::Relative {
        ui.small(
            "Gap before is empty space after the previous allocation. Allocation width includes the content and its inner whitespace.",
        );
    }
    changed
}

fn show_vertical_alignment(ui: &mut egui::Ui, align: &mut VerticalAlign) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Content vertical:");
        for (value, label) in [
            (VerticalAlign::Top, "Top"),
            (VerticalAlign::Center, "Center"),
            (VerticalAlign::Bottom, "Bottom"),
        ] {
            changed |= ui.selectable_value(align, value, label).changed();
        }
    });
    changed
}

/// Freeze the existing unrotated geometry before changing an auto-fit object's
/// angle. Subsequent rotations keep its physical size independent of the slot.
fn set_object_rotation(object: &mut LabelObject, rotation: f32, resolved: &ContentBox) -> bool {
    if rotation == object.rotation {
        return false;
    }
    if object.content_box.is_none() {
        object.content_box = Some(resolved.clone());
    }
    object.rotation = rotation;
    true
}

fn show_layout_properties(
    ui: &mut egui::Ui,
    object: &mut LabelObject,
    resolved: &ContentBox,
    state: &mut AppState,
    positioning: &mut Option<Positioning>,
) -> bool {
    use ptouch_render::object_layout::{LayoutOptions, Placement, WidthSizing};
    let mut changed = false;
    let free = object
        .layout
        .as_ref()
        .is_some_and(|p| p.placement == Placement::Free);
    let legacy = object.layout.is_none();
    if legacy {
        ui.small("Existing layout preserved. Choose positioning to use the new layout rules.");
    }
    ui.horizontal(|ui| {
        if ui.selectable_label(free, "Free positioning").clicked() && !free {
            *positioning = Some(Positioning::Free);
        }
        if ui
            .selectable_label(!free && !legacy, "In sequence")
            .clicked()
            && (free || legacy)
        {
            *positioning = Some(Positioning::Sequence);
        }
    });
    let free = object
        .layout
        .as_ref()
        .is_some_and(|p| p.placement == Placement::Free);
    if free {
        ui.small("Position and size follow the object. Use its blue handles to edit it.");
        ui.label(format!(
            "X {:.2} · Y {:.2} mm",
            object.frame.x_mm, object.frame.y_mm
        ));
        return changed;
    }
    let current = object
        .layout
        .as_ref()
        .map_or(WidthSizing::Fixed, |p| p.width);
    ui.horizontal(|ui| {
        ui.label("Allocation sizing:");
        for (sizing, label) in [
            (WidthSizing::Fixed, "Fixed"),
            (WidthSizing::Content, "Auto width"),
        ] {
            if ui.selectable_label(current == sizing, label).clicked() && current != sizing {
                object.content_box.get_or_insert_with(|| resolved.clone());
                object
                    .layout
                    .get_or_insert(LayoutOptions {
                        placement: Placement::Flow,
                        width: sizing,
                        constrain: false,
                        padding_mm: 0.5,
                    })
                    .width = sizing;
                changed = true;
            }
        }
    });
    if let Some(policy) = &mut object.layout {
        changed |= ui
            .checkbox(&mut policy.constrain, "Keep content inside allocation")
            .changed();
        ui.horizontal(|ui| {
            ui.label("Inner margin:");
            changed |= ui
                .add(
                    crate::widgets::decimal_drag(&mut policy.padding_mm)
                        .range(0.0..=1000.0)
                        .speed(0.1)
                        .suffix(" mm"),
                )
                .changed();
        });
    }
    let old_width = object.frame.width_mm;
    let edited = show_text_box(ui, &mut object.frame, state, false, Some(positioning));
    if edited
        && object.frame.width_mm != old_width
        && let Some(policy) = &mut object.layout
    {
        policy.width = WidthSizing::Fixed;
    }
    changed | edited
}

fn show_allocation_alignment(ui: &mut egui::Ui, alignment: &mut Option<ContentAlignment>) {
    ui.label("Align within allocation");
    for (horizontal, labels) in [
        (true, ["Left", "Center", "Right"]),
        (false, ["Top", "Middle", "Bottom"]),
    ] {
        ui.horizontal(|ui| {
            for (fraction, label) in [0.0, 0.5, 1.0].into_iter().zip(labels) {
                if ui.button(label).clicked() {
                    *alignment = Some(ContentAlignment {
                        horizontal: horizontal.then_some(fraction),
                        vertical: (!horizontal).then_some(fraction),
                    });
                }
            }
        });
    }
}

fn show_content_box(
    ui: &mut egui::Ui,
    object: &mut LabelObject,
    resolved: &ContentBox,
    state: &AppState,
    alignment: &mut Option<ContentAlignment>,
) -> bool {
    let mut changed = false;
    let free = object
        .layout
        .as_ref()
        .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Free);
    let mut auto_fit = object.content_box.is_none();
    if object.layout.is_none()
        && ui
            .add_enabled(
                object.border.is_none(),
                egui::Checkbox::new(&mut auto_fit, "Follow allocation size"),
            )
            .on_disabled_hover_text("Turn off Printed border to follow allocation size.")
            .changed()
    {
        object.content_box = (!auto_fit).then(|| resolved.clone());
        changed = true;
    }
    let mut dimensions = object
        .content_box
        .clone()
        .unwrap_or_else(|| resolved.clone());
    if free && matches!(object.content, ObjectContent::Text { .. }) {
        let policy = object.layout.as_mut().unwrap();
        ui.horizontal(|ui| {
            ui.label("Size:");
            for (value, label) in [
                (
                    ptouch_render::object_layout::WidthSizing::Content,
                    "Auto width",
                ),
                (ptouch_render::object_layout::WidthSizing::Fixed, "Manual"),
            ] {
                changed |= ui
                    .selectable_value(&mut policy.width, value, label)
                    .changed();
            }
        });
    }
    let minimum = 25.4 / f64::from(state.target_dpi());
    let old = dimensions.clone();
    if free {
        let attached = object.scene.as_ref().is_some_and(|s| s.anchor.is_some());
        ui.add_enabled_ui(!attached, |ui| {
            ui.horizontal(|ui| {
                for (label, position, offset, flipped) in [
                    (
                        "X",
                        object.frame.x_mm,
                        &mut dimensions.offset_x_mm,
                        object.flip_h,
                    ),
                    (
                        "Y",
                        object.frame.y_mm,
                        &mut dimensions.offset_y_mm,
                        object.flip_v,
                    ),
                ] {
                    ui.label(label);
                    let mut value = position;
                    if ui
                        .add(
                            crate::widgets::decimal_drag(&mut value)
                                .range(0.0..=10_000.0)
                                .speed(0.1)
                                .suffix(" mm"),
                        )
                        .changed()
                    {
                        *offset += (value - position) * if flipped { -1.0 } else { 1.0 };
                        changed = true;
                    }
                }
            });
        });
        if attached {
            ui.small("Move on the canvas or edit Attachment offsets.");
        }
    }
    ui.add_enabled_ui(!auto_fit, |ui| {
        egui::Grid::new("object_content_dimensions")
            .num_columns(2)
            .show(ui, |ui| {
                for (label, value, min) in [
                    (
                        if matches!(object.content, ObjectContent::Line { .. }) {
                            "Line length"
                        } else {
                            "Object width"
                        },
                        &mut dimensions.width_mm,
                        minimum,
                    ),
                    (
                        if matches!(object.content, ObjectContent::Line { .. }) {
                            "Line width"
                        } else {
                            "Object height"
                        },
                        &mut dimensions.height_mm,
                        minimum,
                    ),
                    ("Offset X", &mut dimensions.offset_x_mm, -10_000.0),
                    ("Offset Y", &mut dimensions.offset_y_mm, -10_000.0),
                ] {
                    if free && label.starts_with("Offset") {
                        continue;
                    }
                    ui.label(label);
                    let response = ui.add(
                        crate::widgets::decimal_drag(value)
                            .speed(0.1)
                            .clamp_existing_to_range(false)
                            .range(min..=10_000.0)
                            .suffix(" mm"),
                    );
                    changed |= response.changed();
                    if response.lost_focus() {
                        // Automatic sizing can immediately replace this value.
                        // Do not replay the consumed text on the next blur frame.
                        ui.data_mut(|data| data.remove_temp::<String>(response.id));
                    }
                    ui.end_row();
                }
            });
    });
    if changed && !auto_fit {
        if object
            .layout
            .as_ref()
            .is_some_and(|p| p.width == ptouch_render::object_layout::WidthSizing::Content)
            && dimensions.width_mm != old.width_mm
            && dimensions.height_mm == old.height_mm
        {
            dimensions.height_mm *= dimensions.width_mm / old.width_mm;
        }
        if free && (dimensions.width_mm != old.width_mm || dimensions.height_mm != old.height_mm) {
            object.layout.as_mut().unwrap().width =
                ptouch_render::object_layout::WidthSizing::Fixed;
        }
        object.content_box = Some(dimensions);
    }
    ui.small(if free {
        "Move, resize and rotate this object within the printable area."
    } else {
        "Content transforms keep the layout allocation fixed."
    });
    if !free {
        show_allocation_alignment(ui, alignment);
    }
    if ui
        .button("Fit inside")
        .on_hover_text("Fit proportionally to the available area; text uses automatic font size")
        .clicked()
    {
        let c = object.content_box.get_or_insert_with(|| resolved.clone());
        let (sin, cos) = f64::from(object.rotation).to_radians().sin_cos();
        let w = c.width_mm * cos.abs() + c.height_mm * sin.abs();
        let h = c.width_mm * sin.abs() + c.height_mm * cos.abs();
        let pad = object.layout.as_ref().map_or(0.0, |p| p.padding_mm) * 2.0;
        let available_w = if free {
            10_000.0
        } else {
            object.frame.width_mm - pad
        };
        let available_h = if free {
            state.printable_height_mm() - pad
        } else {
            object.frame.height_mm - pad
        };
        let ratio = (available_w / w).min(available_h / h).clamp(0.01, 1.0);
        c.width_mm *= ratio;
        c.height_mm *= ratio;
        c.offset_x_mm = 0.0;
        c.offset_y_mm = 0.0;
        if let ObjectContent::Text {
            content,
            font_size,
            styles,
            ..
        } = &mut object.content
            && let Some(size) = *font_size
        {
            if !styles.is_empty() {
                *styles = ptouch_render::styled_text::style_regions(content, styles)
                    .into_iter()
                    .map(|(range, mut style)| {
                        style.font_size = Some(style.font_size.unwrap_or(size) * ratio as f32);
                        ptouch_render::styled_text::StyleSpan {
                            start: range.start,
                            end: range.end,
                            style,
                        }
                    })
                    .collect();
            }
            *font_size = None;
        }
        changed = true;
    }
    changed
}

/// Objects have independent content geometry and a fixed template slot.
fn show_object_properties(
    ui: &mut egui::Ui,
    object: &mut LabelObject,
    state: &mut AppState,
    alignment: &mut Option<ContentAlignment>,
    positioning: &mut Option<Positioning>,
) -> bool {
    // Resolve this before drawing any controls: the first transform of an old
    // auto-fit object must use the geometry from its previous angle.
    let resolved = state
        .selected_element
        .and_then(|index| state.resolved_content_box(index))
        .unwrap_or(ContentBox {
            width_mm: object.frame.width_mm,
            height_mm: object.frame.height_mm,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
    let attached = object
        .scene
        .as_ref()
        .is_some_and(|scene| scene.anchor.is_some());
    let layout = state.canvas_edit_target == CanvasEditTarget::Slot;
    if !attached
        && ui
            .button(if layout {
                "Done adjusting layout"
            } else {
                "Adjust layout…"
            })
            .clicked()
    {
        state.select_canvas_target(state.selected_element.map(|element| CanvasTarget {
            element,
            edit: if layout {
                CanvasEditTarget::Content
            } else {
                CanvasEditTarget::Slot
            },
        }));
    }
    let mut changed = match state.canvas_edit_target {
        CanvasEditTarget::Content => {
            let mut changed = show_content_box(ui, object, &resolved, state, alignment);
            if !matches!(
                object.content,
                ObjectContent::Line { .. } | ObjectContent::Shape { .. }
            ) {
                changed |= show_vertical_alignment(ui, &mut object.frame.vertical_align);
            }
            changed
        }
        CanvasEditTarget::Slot => {
            ui.label("Position and spacing");
            let changed = show_layout_properties(ui, object, &resolved, state, positioning);
            ui.small("Allocation width and gap affect following objects. Rotation changes only this object's content.");
            changed
        }
    };
    if state.canvas_edit_target == CanvasEditTarget::Slot {
        return changed;
    }
    ui.separator();
    match &mut object.content {
        ObjectContent::Text {
            content,
            font_size,
            align,
            line_spacing,
            styles,
        } => {
            ui.label("Text Content:");
            let editor_id = egui::Id::new((
                "inline_editor",
                state.editor_generation,
                state.selected_element,
            ));
            changed |= crate::rich_text_editor::show(
                ui,
                content,
                styles,
                editor_id,
                &state.font_name,
                font_size.unwrap_or(12.0),
                &state.available_fonts,
            );
            if content.contains("{{") {
                ui.small("Fixed allocations keep template positions unchanged.");
            }
            let old_base_size = *font_size;
            changed |= show_font_properties(ui, font_size, align, true, state);
            if let Some(old) = old_base_size
                && font_size.is_none()
                && !styles.is_empty()
            {
                // Freeze inherited point sizes before auto-fitting the whole block.
                *styles = ptouch_render::styled_text::style_regions(content, styles)
                    .into_iter()
                    .map(|(range, mut style)| {
                        style.font_size = Some(style.font_size.unwrap_or(old));
                        ptouch_render::styled_text::StyleSpan {
                            start: range.start,
                            end: range.end,
                            style,
                        }
                    })
                    .collect();
                changed = true;
            }
            ui.small("Auto font size scales all inline sizes together.");
            ui.horizontal(|ui| {
                ui.label("Line spacing:");
                changed |= ui.add(crate::widgets::decimal_drag(line_spacing)
                    .range(ptouch_render::text::LINE_SPACING_RANGE)
                    .speed(0.05).min_decimals(2).max_decimals(2).suffix(" ×"))
                    .on_hover_text("Baseline distance as a multiple of font size. 1.20 keeps the original spacing.")
                    .changed();
            });
        }
        ObjectContent::Image {
            path,
            image_data,
            bitmap,
            preserve_alpha,
            coverage,
        } => {
            ui.label("Image");
            let filename = path.as_ref().and_then(|path| path.file_name()).map_or_else(
                || "embedded".to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            ui.label(format!("File: {filename}"));
            if let Some(bitmap) = bitmap {
                ui.small(format!(
                    "Original: {} x {} px",
                    bitmap.width(),
                    bitmap.height()
                ));
            }
            ui.small(
                "The image keeps its aspect ratio inside the object box, then rotates with it.",
            );
            if ui.add_enabled(state.is_free_layout(), egui::Checkbox::new(preserve_alpha, "Use image transparency"))
                .on_hover_text("Transparent pixels pass through; opaque white covers lower layers. Partial alpha becomes monochrome coverage.")
                .changed() {
                *bitmap = None; *coverage = None; changed = true;
            }
            if let Some(path) = path.clone()
                && ui.button("Reload").clicked()
            {
                if load_image_into(&path, image_data, bitmap) {
                    *coverage = None;
                    if *preserve_alpha {
                        *bitmap = None;
                    }
                    changed = true;
                } else {
                    state.status_message = format!("Image load error: {}", path.display());
                }
            }
            if ui.button("Change File...").clicked()
                && let Some(new_path) = crate::widgets::image_file_dialog().pick_file()
            {
                if load_image_into(&new_path, image_data, bitmap) {
                    *path = Some(new_path);
                    *coverage = None;
                    if *preserve_alpha {
                        *bitmap = None;
                    }
                    changed = true;
                } else {
                    state.status_message = format!("Image load error: {}", new_path.display());
                }
            }
        }
        ObjectContent::Line { pattern } => {
            ui.label("Printed line");
            changed |= show_stroke_pattern(ui, pattern);
            ui.small("Printed guide only; does not operate the cutter.");
        }
        ObjectContent::Shape { style } => {
            ui.label("Shape");
            egui::ComboBox::from_id_salt("shape_kind")
                .selected_text(match style.kind {
                    ptouch_render::shape::ShapeKind::Rectangle => "Rectangle",
                    ptouch_render::shape::ShapeKind::Ellipse => "Ellipse",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(
                            &mut style.kind,
                            ptouch_render::shape::ShapeKind::Rectangle,
                            "Rectangle",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut style.kind,
                            ptouch_render::shape::ShapeKind::Ellipse,
                            "Ellipse",
                        )
                        .changed();
                });
            if style.kind == ptouch_render::shape::ShapeKind::Ellipse
                && ui.button("Make circle").clicked()
                && let Some(c) = &mut object.content_box
            {
                let side = c.width_mm.min(c.height_mm);
                c.width_mm = side;
                c.height_mm = side;
                changed = true;
            }
            ui.horizontal(|ui| {
                ui.label("Fill:");
                for (fill, label) in [
                    (ptouch_render::shape::ShapeFill::None, "None"),
                    (ptouch_render::shape::ShapeFill::Ink, "Ink"),
                    (ptouch_render::shape::ShapeFill::Paper, "Tape"),
                ] {
                    changed |= ui.selectable_value(&mut style.fill, fill, label).changed();
                }
            });
            ui.horizontal(|ui| {
                ui.label("Stroke:");
                changed |= ui
                    .add(
                        crate::widgets::decimal_drag(&mut style.stroke_width_mm)
                            .range(0.0..=1000.0)
                            .speed(0.1)
                            .min_decimals(2)
                            .suffix(" mm"),
                    )
                    .changed();
            });
            changed |= show_stroke_pattern(ui, &mut style.pattern);
            ui.small("Tape fill covers lower layers. None keeps the inside transparent.");
        }
        ObjectContent::CutMark => {
            ui.label("Cut mark");
            ui.small("A printed guide only; it does not trigger the cutter.");
        }
        ObjectContent::Padding => {
            ui.label("Blank allocation (converted padding)");
            ui.small("Keeps its extent and can anchor relative objects. Use positions or gaps for new spacing.");
        }
    }
    if matches!(
        object.content,
        ObjectContent::Text { .. } | ObjectContent::Image { .. }
    ) {
        ui.separator();
        let mut enabled = object.border.is_some();
        if ui.checkbox(&mut enabled, "Printed border").changed() {
            object.border = enabled.then(Default::default);
            if enabled {
                object.content_box.get_or_insert_with(|| resolved.clone());
            }
            changed = true;
        }
        if let Some(border) = &mut object.border {
            let before = border.clone();
            for (label, value, min) in [
                ("Border width", &mut border.width_mm, 0.01),
                ("Border spacing", &mut border.padding_mm, 0.0),
            ] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(
                        crate::widgets::decimal_drag(value)
                            .range(min..=1000.0)
                            .clamp_existing_to_range(false)
                            .speed(0.1)
                            .suffix(" mm"),
                    );
                });
            }
            changed |= show_stroke_pattern(ui, &mut border.pattern);
            changed |= *border != before;
            ui.small("Follows this object. Spacing is inside the printed border.");
            if let (Some(c), Ok(inset)) = (&object.content_box, border.inset_mm(state.target_dpi()))
                && (c.width_mm <= 2.0 * inset || c.height_mm <= 2.0 * inset)
            {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Enlarge the object or reduce border spacing.",
                );
            }
        }
    }
    ui.separator();
    ui.label("Object rotation:");
    let mut rotation = object.rotation;
    let mut rotation_changed = ui
        .add(
            crate::widgets::decimal_drag(&mut rotation)
                .speed(1.0)
                .clamp_existing_to_range(false)
                .range(-360.0..=360.0)
                .suffix(" deg"),
        )
        .changed();
    ui.horizontal(|ui| {
        for degrees in [0.0_f32, 45.0, 90.0, 180.0, 270.0] {
            rotation_changed |= ui
                .selectable_value(&mut rotation, degrees, format!("{}", degrees as i32))
                .changed();
        }
    });
    if rotation_changed {
        changed |= set_object_rotation(object, rotation, &resolved);
    }
    changed |= show_flip_controls(ui, &mut object.flip_h, &mut object.flip_v);
    changed
}

fn show_stroke_pattern(
    ui: &mut egui::Ui,
    pattern: &mut ptouch_render::stroke::StrokePattern,
) -> bool {
    use ptouch_render::stroke::PatternKind;
    let before = pattern.clone();
    egui::ComboBox::from_id_salt("stroke_pattern")
        .selected_text(match pattern.kind {
            PatternKind::Solid => "Solid",
            PatternKind::Dashed => "Dashed",
            PatternKind::Dotted => "Dotted",
        })
        .show_ui(ui, |ui| {
            for (kind, label) in [
                (PatternKind::Solid, "Solid"),
                (PatternKind::Dashed, "Dashed"),
                (PatternKind::Dotted, "Dotted"),
            ] {
                ui.selectable_value(&mut pattern.kind, kind, label);
            }
        });
    for (name, value, visible) in [
        (
            "Dash length",
            &mut pattern.dash_mm,
            pattern.kind == PatternKind::Dashed,
        ),
        (
            "Gap",
            &mut pattern.gap_mm,
            pattern.kind != PatternKind::Solid,
        ),
    ] {
        if visible {
            ui.horizontal(|ui| {
                ui.label(name);
                ui.add(
                    crate::widgets::decimal_drag(value)
                        .range(0.01..=1000.0)
                        .speed(0.1)
                        .suffix(" mm"),
                );
            });
        }
    }
    *pattern != before
}

/// Read and decode an image file, updating the embedded bytes and render cache.
/// Returns true on success.
fn load_image_into(
    path: &Path,
    image_data: &mut Vec<u8>,
    bitmap: &mut Option<ptouch_render::bitmap::LabelBitmap>,
) -> bool {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Image read failed: {}", e);
            return false;
        }
    };
    match image_loader::load_image_from_reader(
        Cursor::new(&bytes),
        &image_loader::ImageLoadOptions::default(),
    ) {
        Ok(bmp) => {
            *image_data = bytes;
            *bitmap = Some(bmp);
            true
        }
        Err(e) => {
            error!("Image decode failed: {}", e);
            false
        }
    }
}

/// Show properties for an image element. Returns true if changed.
fn show_image_properties(ui: &mut egui::Ui, props: ImageProps, state: &mut AppState) -> bool {
    let ImageProps {
        path,
        image_data,
        bitmap,
        rotation,
        target_height,
        flip_h,
        flip_v,
    } = props;
    let mut changed = false;

    ui.label("Image");
    ui.add_space(4.0);

    let file_label = path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "embedded".to_string());
    ui.label(format!("File: {}", file_label));

    if let &mut Some(ref bmp) = bitmap {
        ui.label(format!("Original: {} x {} px", bmp.width(), bmp.height()));
    }

    ui.add_space(4.0);

    // Reload re-reads the original file when its path is still known.
    if let Some(p) = path.clone()
        && ui.button("Reload").clicked()
        && load_image_into(&p, image_data, bitmap)
    {
        info!("Reloaded image: {}", p.display());
        changed = true;
    }

    if ui.button("Change File...").clicked()
        && let Some(new_path) = crate::widgets::image_file_dialog().pick_file()
        && load_image_into(&new_path, image_data, bitmap)
    {
        info!("Changed image to: {}", new_path.display());
        *path = Some(new_path);
        changed = true;
    }

    ui.add_space(8.0);

    // Height sizing (auto = fit to tape, manual = user-specified)
    let mut use_auto = target_height.is_none();
    if ui.checkbox(&mut use_auto, "Auto height").changed() {
        if use_auto {
            *target_height = None;
        } else {
            *target_height = Some(state.tape_width_px);
        }
        changed = true;
    }

    if let Some(h) = target_height {
        ui.horizontal(|ui| {
            ui.label("Height:");
            if ui
                .add(
                    crate::widgets::decimal_drag(h)
                        .speed(1.0)
                        .clamp_existing_to_range(false)
                        .range(1..=1000),
                )
                .changed()
            {
                changed = true;
            }
            ui.label("px");
        });
    }

    ui.add_space(4.0);

    // Rotation
    ui.label("Rotation:");
    ui.horizontal(|ui| {
        if ui
            .add(
                crate::widgets::decimal_drag(rotation)
                    .speed(1.0)
                    .clamp_existing_to_range(false)
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
        if resp.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && let Ok(val) = state.rotation_input.parse::<f32>()
        {
            let clamped = val.clamp(-360.0, 360.0);
            *rotation = clamped;
            state.rotation_input = format!("{}", clamped);
            changed = true;
        }
    });

    ui.add_space(4.0);
    changed |= show_flip_controls(ui, flip_h, flip_v);

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
            .add(
                crate::widgets::decimal_drag(pixels)
                    .speed(1.0)
                    .clamp_existing_to_range(false)
                    .range(1..=500),
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_width_then_clicking_center_does_not_replay_the_blurred_value() {
        use ptouch_render::object_layout::{self, LayoutOptions, Placement, WidthSizing};

        fn frame(
            context: &egui::Context,
            state: &mut AppState,
            events: Vec<egui::Event>,
        ) -> Vec<(String, egui::Rect)> {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(280.0, 1200.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show_properties(ui, state),
            );
            output.textures_delta.clear();
            let mut resolved = state.document();
            object_layout::resolve(
                &mut resolved,
                state.scene_measurement_renderer.as_mut().unwrap(),
            )
            .unwrap();
            state.elements = resolved.elements;
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.galley.rect.translate(text.pos.to_vec2()),
                    )),
                    _ => None,
                })
                .collect()
        }
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        for scene in [false, true] {
            let mut state = AppState {
                document_version: if scene { 12 } else { 7 },
                selected_element: Some(0),
                ..Default::default()
            };
            let mut object = LabelObject::text(
                "PORT 01",
                TextBox {
                    x_mm: 1.0,
                    height_mm: 10.0,
                    ..Default::default()
                },
            );
            object.layout = Some(LayoutOptions {
                placement: Placement::Flow,
                width: WidthSizing::Content,
                constrain: false,
                padding_mm: 0.0,
            });
            object.content_box = Some(ContentBox {
                width_mm: 13.8,
                height_mm: 4.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            });
            if scene {
                object.scene = Some(Box::new(ptouch_render::scene::SceneOptions {
                    id: 1,
                    layer: 0,
                    anchor: None,
                }));
            }
            object_layout::resolve_object(
                &mut object,
                state.document_version,
                state.design_dpi,
                &state.font_name,
                state
                    .scene_measurement_renderer
                    .get_or_insert_with(ptouch_render::text::TextRenderer::new),
            )
            .unwrap();
            state.elements = vec![LabelElement::Object(object.clone())];
            let context = egui::Context::default();
            let shapes = frame(&context, &mut state, vec![]);
            let width_label = shapes
                .iter()
                .find(|(text, _)| text == "Object width")
                .unwrap()
                .1;
            let width = shapes
                .iter()
                .find(|(text, rect)| {
                    text.ends_with(" mm")
                        && rect.min.x > width_label.max.x
                        && (rect.center().y - width_label.center().y).abs() < 3.0
                })
                .unwrap()
                .1;
            let center = shapes.iter().find(|(text, _)| text == "Center").unwrap().1;
            frame(&context, &mut state, click(width.center()));
            frame(&context, &mut state, vec![]);
            frame(&context, &mut state, vec![egui::Event::Text("12.0".into())]);
            assert_eq!(
                state.elements[0].as_object().unwrap().content_box,
                object.content_box
            );
            frame(&context, &mut state, click(center.center()));
            for _ in 0..4 {
                frame(&context, &mut state, vec![]);
            }
            let result = state.elements[0].as_object().unwrap();
            let content = result.content_box.as_ref().unwrap();
            assert_eq!(content.offset_x_mm, 0.0);
            assert_eq!(result.frame.width_mm, content.width_mm);
            // One proportional edit, with no second shrink from a stale buffer.
            assert!((content.height_mm - 4.0 * 12.0 / object.frame.width_mm).abs() < 1e-9);
        }
    }

    #[test]
    fn attached_properties_normalize_a_stale_layout_target_without_editing_the_document() {
        let mut state = AppState::default();
        state.add_text();
        assert!(state.add_overlay_text(0));
        state.canvas_edit_target = CanvasEditTarget::Slot;
        state.needs_rerender = false;
        let before = state.document();
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 1600.0),
                )),
                ..Default::default()
            },
            |ui| show_properties(ui, &mut state),
        );
        output.textures_delta.clear();
        assert_eq!(state.canvas_edit_target, CanvasEditTarget::Content);
        assert_eq!(state.document(), before);
        assert!(!state.needs_rerender);
        assert!(!output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text)
                if text.galley.text() == "Adjust layout…"
                    || text.galley.text() == "Done adjusting layout"
        )));
    }

    #[test]
    fn viewing_properties_does_not_clamp_an_existing_design() {
        let frame = TextBox {
            width_mm: 0.08,
            height_mm: 20.0,
            y_mm: 3.0,
            ..TextBox::default()
        };
        let mut object = LabelObject::text("Text", frame.clone());
        object.rotation = 370.0;
        if let ObjectContent::Text { font_size, .. } = &mut object.content {
            *font_size = Some(300.0);
        }
        let mut independent = object.clone();
        independent.content_box = Some(ContentBox {
            width_mm: 0.08,
            height_mm: 20.0,
            offset_x_mm: -12_000.0,
            offset_y_mm: 15_000.0,
        });
        for (version, element) in [
            (4, LabelElement::Object(independent)),
            (3, LabelElement::Object(object)),
            (
                2,
                LabelElement::Text {
                    content: "Legacy".into(),
                    font_size: Some(300.0),
                    align: TextAlign::Left,
                    rotation: 370.0,
                    flip_h: false,
                    flip_v: false,
                    text_box: Some(frame),
                },
            ),
            (
                1,
                LabelElement::Text {
                    content: "Legacy natural".into(),
                    font_size: Some(300.0),
                    align: TextAlign::Left,
                    rotation: 370.0,
                    flip_h: false,
                    flip_v: false,
                    text_box: None,
                },
            ),
        ] {
            for target in [CanvasEditTarget::Content, CanvasEditTarget::Slot] {
                let mut state = AppState {
                    document_version: version,
                    elements: vec![element.clone()],
                    selected_element: Some(0),
                    canvas_edit_target: target,
                    needs_rerender: false,
                    ..AppState::default()
                };
                let before = state.document().to_toml_string().unwrap();
                let context = egui::Context::default();
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    show_properties(ui, &mut state);
                });
                output.textures_delta.clear();
                assert_eq!(state.document().to_toml_string().unwrap(), before);
                assert_eq!(state.document_version, version);
                assert!(!state.needs_rerender);
            }
        }
    }

    #[test]
    fn rotating_auto_fit_pins_previous_geometry_then_preserves_it() {
        let mut object = LabelObject::text("PORT1", TextBox::default());
        let old_slot = object.frame.clone();
        let previous = ContentBox {
            width_mm: 8.7,
            height_mm: 3.9,
            offset_x_mm: 0.5,
            offset_y_mm: -0.2,
        };
        assert!(!set_object_rotation(&mut object, 0.0, &previous));
        assert!(object.content_box.is_none());
        assert!(set_object_rotation(&mut object, 45.0, &previous));
        assert_eq!(object.content_box, Some(previous.clone()));
        let stale_preview = ContentBox {
            width_mm: 1.0,
            ..previous.clone()
        };
        assert!(set_object_rotation(&mut object, 90.0, &stale_preview));
        assert_eq!(object.content_box, Some(previous));
        assert_eq!(object.frame, old_slot);
    }

    #[test]
    fn changing_position_modes_keeps_exact_physical_position() {
        let mut frame = TextBox {
            x_mm: 18.0,
            ..TextBox::default()
        };
        assert!(switch_position_mode(&mut frame, PositionMode::Relative, 15.0).unwrap());
        assert_eq!(frame.gap_mm, 3.0);
        assert!(switch_position_mode(&mut frame, PositionMode::Absolute, 15.0).unwrap());
        assert_eq!(frame.x_mm, 18.0);
    }

    #[test]
    fn overlapping_frame_cannot_silently_jump_when_made_relative() {
        let mut frame = TextBox {
            x_mm: 12.0,
            ..TextBox::default()
        };
        let original = frame.clone();
        assert!(switch_position_mode(&mut frame, PositionMode::Relative, 15.0).is_err());
        assert_eq!(frame, original);
    }
}

#[cfg(test)]
mod multiple_selection_panel_tests {
    use super::*;
    fn frame(
        ctx: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| show_properties(ui, state),
        );
        output.textures_delta.clear();
        output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().into(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect()
    }
    #[test]
    fn common_dimensions_stay_open_when_history_changes_selection_revision() {
        let ctx = egui::Context::default();
        ctx.style_mut_of(egui::Theme::Dark, |style| style.animation_time = 0.0);
        ctx.style_mut_of(egui::Theme::Light, |style| style.animation_time = 0.0);
        let mut state = AppState::default();
        state.add_text();
        state.add_text();
        state.set_selected_indices(&[0, 1], Some(0));
        let text = frame(&ctx, &mut state, Vec::new());
        let pos = text
            .iter()
            .find(|(text, _)| text == "Content dimensions")
            .unwrap()
            .1
            .center();
        frame(
            &ctx,
            &mut state,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        let text = frame(&ctx, &mut state, Vec::new());
        assert!(text.iter().any(|(text, _)| text == "Width"));
        let document = state.document();
        state.restore_document(document);
        state.set_selected_indices(&[0, 1], Some(0));
        let text = frame(&ctx, &mut state, Vec::new());
        assert!(text.iter().any(|(text, _)| text == "Width"));
    }
}
