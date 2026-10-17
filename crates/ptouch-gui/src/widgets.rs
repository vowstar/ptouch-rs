// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Custom egui widgets and shared helpers.

/// Parse an ungrouped decimal using either decimal separator. Never accept a
/// numeric prefix, ambiguous grouping, non-finite values, or internal spaces.
pub fn parse_decimal(text: &str) -> Option<f64> {
    let text = text.trim().replace('−', "-");
    if text.chars().any(char::is_whitespace)
        || text.contains(',') && text.contains('.')
        || text.matches(',').count() > 1
    {
        return None;
    }
    text.replace(',', ".")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Commit keyboard edits as a whole; dragging still updates immediately.
pub fn decimal_drag<N: egui::emath::Numeric>(value: &mut N) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .custom_parser(parse_decimal)
        .update_while_editing(false)
        .clamp_existing_to_range(false)
}

/// Build an image file dialog with per-format filters.
///
/// The first filter is "All Images" (default), followed by individual
/// format groups so the user can narrow down if needed.
pub fn image_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .add_filter(
            "All Images",
            &[
                "png", "jpg", "jpeg", "gif", "bmp", "tiff", "tif", "webp", "ico", "pnm", "tga",
                "qoi", "svg", "svgz",
            ],
        )
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .add_filter("SVG", &["svg", "svgz"])
        .add_filter("GIF", &["gif"])
        .add_filter("BMP", &["bmp"])
        .add_filter("TIFF", &["tiff", "tif"])
        .add_filter("WebP", &["webp"])
        .add_filter("ICO", &["ico"])
        .add_filter("QOI", &["qoi"])
        .add_filter("PNM", &["pnm"])
        .add_filter("TGA", &["tga"])
}

/// Build a file dialog filtered to native layouts and imported LBX files.
pub fn layout_file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Label layouts", &["ptl", "lbx"])
}

/// iOS-style toggle switch widget.
///
/// Based on the egui demo toggle_switch example.
pub fn toggle(on: &mut bool) -> impl egui::Widget + '_ {
    move |ui: &mut egui::Ui| toggle_ui(ui, on)
}

fn toggle_ui(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, "")
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact_selectable(&response, *on);
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();
        ui.painter().rect(
            rect,
            radius,
            visuals.bg_fill,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        ui.painter()
            .circle(center, 0.75 * radius, visuals.bg_fill, visuals.fg_stroke);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_input_accepts_both_separators_but_rejects_ambiguous_values() {
        for (input, expected) in [
            ("14,5", 14.5),
            ("14.5", 14.5),
            (" −0,25 ", -0.25),
            (",5", 0.5),
            ("1.2e2", 120.0),
        ] {
            assert_eq!(parse_decimal(input), Some(expected), "{input}");
        }
        for input in [
            "14,5.0",
            "1,234,567",
            "1 234",
            "14foo",
            "NaN",
            "inf",
            "1e999",
            "",
            "-",
        ] {
            assert_eq!(parse_decimal(input), None, "{input}");
        }
    }

    #[test]
    fn keyboard_edits_commit_whole_values_on_enter_or_blur() {
        for (text, expected) in [("14,5", 14.5), ("14.5", 14.5), ("14,5.0", 8.125)] {
            for enter in [false, true] {
                let context = egui::Context::default();
                let mut value = 8.125;
                let mut rect = egui::Rect::NOTHING;
                let mut frame = |events| {
                    let mut output = context.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(800.0, 600.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            rect = ui.add(decimal_drag(&mut value).suffix(" mm")).rect;
                        },
                    );
                    output.textures_delta.clear();
                    (rect, value)
                };
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
                let (rect, _) = frame(vec![]);
                frame(click(rect.center()));
                frame(vec![]);
                assert_eq!(frame(vec![egui::Event::Text(text.into())]).1, 8.125);
                if enter {
                    frame(vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                } else {
                    frame(click(egui::pos2(300.0, 300.0)));
                }
                assert_eq!(frame(vec![]).1, expected, "{text}, enter={enter}");
            }
        }
    }
}
