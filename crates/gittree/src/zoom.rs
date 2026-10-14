//! Interface zoom (FR-SET-04). Views size things with this `px`, which is
//! a length in `rem`: GPUI scales every `rem` by the window's rem size, so
//! one setting zooms the whole interface. At 100% it equals `gpui::px`.

use gpui_kit::{App, Pixels, Rems, rems};

/// The rem size at 100%.
pub const BASE: f32 = 16.;

/// `value` pixels at 100%, scaled with the zoom.
pub fn px(value: f32) -> Rems {
    rems(value / BASE)
}

/// The zoom set in Settings, 1.0 at 100%.
pub fn level(cx: &App) -> f32 {
    cx.try_global::<crate::app_state::AppStateHandle>()
        .map_or(1., |app| clamp(app.0.read(cx).settings.zoom))
}

/// `value` pixels at 100%, as real pixels at the current zoom. For the few
/// places that take `Pixels`, such as a dialog's width.
pub fn scaled(value: f32, cx: &App) -> Pixels {
    gpui_kit::px(value * level(cx))
}

/// The allowed zoom range, and the step of Zoom in and Zoom out.
pub const MIN: f32 = 0.5;
pub const MAX: f32 = 3.0;
pub const STEP: f32 = 0.1;

/// A zoom kept in range and rounded to the step.
pub fn clamp(zoom: f32) -> f32 {
    ((zoom / STEP).round() * STEP).clamp(MIN, MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zoom_stays_in_range_on_its_steps() {
        assert_eq!(clamp(1.0), 1.0);
        assert_eq!(clamp(0.2), MIN);
        assert_eq!(clamp(9.0), MAX);
        assert!((clamp(1.0 + STEP) - 1.1).abs() < 1e-6);
        assert!((clamp(1.26) - 1.3).abs() < 1e-6);
        assert_eq!(px(16.), rems(1.));
    }
}
