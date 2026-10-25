// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Canvas view transforms in logical points, independent of print resolution.

pub fn scroll_area(offset: Option<egui::Vec2>) -> egui::ScrollArea {
    let area = egui::ScrollArea::both();
    match offset {
        Some(offset) => area.scroll_offset(offset.max(egui::Vec2::ZERO)),
        None => area,
    }
}

/// Ignore the tail of wheel smoothing after a blocked gesture or another panel.
pub fn accepts_input(ctx: &egui::Context, id: egui::Id, blocked: bool, inside: bool) -> bool {
    let (delta, fresh) = ctx.input(|input| {
        let fresh = input.events.iter().any(|event| match event {
            egui::Event::Zoom(factor) => factor.is_finite() && *factor > 0.0 && *factor != 1.0,
            egui::Event::MouseWheel {
                modifiers, delta, ..
            } => modifiers.ctrl && *delta != egui::Vec2::ZERO,
            _ => false,
        });
        (input.zoom_delta(), fresh)
    });
    ctx.data_mut(|data| {
        let mut suppressed = data.get_temp::<bool>(id).unwrap_or(false);
        let accepted = if delta == 1.0 {
            suppressed = false;
            false
        } else if blocked || !inside {
            suppressed = true;
            false
        } else {
            if fresh {
                suppressed = false;
            }
            !suppressed
        };
        data.insert_temp(id, suppressed);
        accepted
    })
}

pub fn anchored_offset(
    old_offset: egui::Vec2,
    pointer_from_origin: egui::Vec2,
    old_zoom: f32,
    new_zoom: f32,
) -> egui::Vec2 {
    (pointer_from_origin + old_offset) * (new_zoom / old_zoom) - pointer_from_origin
}

pub fn reveal_offset(
    offset: egui::Vec2,
    viewport_size: egui::Vec2,
    selected: egui::Rect,
) -> Option<egui::Vec2> {
    let visible = egui::Rect::from_min_size(egui::Pos2::ZERO + offset, viewport_size);
    if visible.intersects(selected) {
        None
    } else {
        Some((selected.center().to_vec2() - viewport_size / 2.0).max(egui::Vec2::ZERO))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_and_inverse_keep_the_pointer_on_the_same_document_point() {
        let pointer = egui::vec2(230.0, 40.0);
        for zoom in [0.05, 1.0, 4.0, 20.0] {
            let old = egui::vec2(120.0, 30.0);
            let new = anchored_offset(old, pointer, zoom, zoom * 1.25);
            let before = (pointer + old) / zoom;
            let after = (pointer + new) / (zoom * 1.25);
            assert!((before - after).length() < 0.001);
            assert!((anchored_offset(new, pointer, zoom * 1.25, zoom) - old).length() < 0.001);
        }
    }

    #[test]
    fn selection_reveal_keeps_partly_visible_items_in_place() {
        let offset = egui::vec2(100.0, 0.0);
        let viewport = egui::vec2(300.0, 200.0);
        let partly_visible =
            egui::Rect::from_min_size(egui::pos2(390.0, 10.0), egui::vec2(50.0, 50.0));
        assert_eq!(reveal_offset(offset, viewport, partly_visible), None);
        let outside = partly_visible.translate(egui::vec2(500.0, 0.0));
        let revealed = reveal_offset(offset, viewport, outside).unwrap();
        assert!(
            egui::Rect::from_min_size(egui::Pos2::ZERO + revealed, viewport).contains_rect(outside)
        );
    }
}
