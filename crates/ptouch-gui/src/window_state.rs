// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Recover unusable initial geometry without resetting the saved editor state.

const MINIMUM: egui::Vec2 = egui::vec2(800.0, 500.0);
const DEFAULT: egui::Vec2 = egui::vec2(1200.0, 700.0);

fn usable_size(size: egui::Vec2, monitor: Option<egui::Vec2>) -> (egui::Vec2, egui::Vec2) {
    let available = monitor
        .filter(|size| size.is_finite() && size.x > 0.0 && size.y > 0.0)
        .map(|size| (size - egui::vec2(32.0, 64.0)).max(egui::Vec2::splat(1.0)))
        .unwrap_or(egui::Vec2::splat(f32::INFINITY));
    let minimum = MINIMUM.min(available);
    let size = if size.is_finite() && size.x > 0.0 && size.y > 0.0 {
        size
    } else {
        DEFAULT
    };
    (size.max(minimum).min(available), minimum)
}

pub fn recover_startup(ctx: &egui::Context) {
    let id = egui::Id::new("startup_window_checked");
    if ctx.data(|data| data.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    let viewport = ctx.input(|input| input.viewport().clone());
    if viewport.minimized == Some(true) {
        return;
    }
    let Some(rect) = viewport.inner_rect else {
        return;
    };
    if viewport.maximized != Some(true) && viewport.fullscreen != Some(true) {
        let (size, minimum) = usable_size(rect.size(), viewport.monitor_size);
        // A small display must also be allowed to override the normal minimum.
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(minimum));
        if (size - rect.size()).length_sq() > 1.0 || !rect.is_finite() {
            log::info!(
                "Recovered initial window size to {} by {} points",
                size.x,
                size.y
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.request_repaint();
        }
    }
    ctx.data_mut(|data| data.insert_temp(id, true));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_and_tiny_saved_sizes_recover_in_logical_points() {
        for size in [
            egui::vec2(64.0, 64.0),
            egui::vec2(0.0, -1.0),
            egui::vec2(f32::NAN, 2.0),
        ] {
            let (recovered, _) = usable_size(size, Some(egui::vec2(1920.0, 1080.0)));
            assert!(recovered.x >= 800.0 && recovered.y >= 500.0);
            assert!(recovered.is_finite());
        }
        for size in [egui::vec2(800.0, 500.0), egui::vec2(1350.0, 750.0)] {
            assert_eq!(usable_size(size, Some(egui::vec2(1920.0, 1080.0))).0, size);
        }
    }

    #[test]
    fn small_or_changed_monitors_bound_the_recovery_and_minimum() {
        let monitor = egui::vec2(640.0, 480.0);
        let (size, minimum) = usable_size(DEFAULT, Some(monitor));
        assert!(size.x < monitor.x && size.y < monitor.y);
        assert_eq!(size, minimum);
        assert_eq!(usable_size(MINIMUM, None).0, MINIMUM);
        assert_eq!(
            usable_size(DEFAULT, Some(egui::Vec2::splat(f32::NAN))).0,
            DEFAULT
        );
    }

    fn frame(
        ctx: &egui::Context,
        size: egui::Vec2,
        minimized: bool,
        maximized: bool,
    ) -> Vec<egui::ViewportCommand> {
        let mut input = egui::RawInput::default();
        let viewport = input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap();
        viewport.inner_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
        viewport.monitor_size = Some(egui::vec2(1920.0, 1080.0));
        viewport.minimized = Some(minimized);
        viewport.maximized = Some(maximized);
        let mut output = ctx.run_ui(input, |ui| recover_startup(ui.ctx()));
        output.textures_delta.clear();
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .filter(|command| {
                matches!(
                    command,
                    egui::ViewportCommand::InnerSize(_) | egui::ViewportCommand::MinInnerSize(_)
                )
            })
            .cloned()
            .collect()
    }

    #[test]
    fn correction_runs_once_and_waits_until_the_window_is_restored() {
        let ctx = egui::Context::default();
        let tiny = egui::vec2(64.0, 64.0);
        assert!(frame(&ctx, tiny, true, false).is_empty());
        assert!(frame(&ctx, tiny, false, false).iter().any(
            |command| matches!(command, egui::ViewportCommand::InnerSize(size) if *size == MINIMUM)
        ));
        assert!(frame(&ctx, tiny, false, false).is_empty());
        let ctx = egui::Context::default();
        assert!(frame(&ctx, tiny, false, true).is_empty());
    }
}
