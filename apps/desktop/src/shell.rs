use gpui::{App, Bounds, Pixels, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

pub const WINDOW_PLACEMENT_SETTING: &str = "ui.window_placement";
pub const MIN_WINDOW_WIDTH: f32 = 920.0;
pub const MIN_WINDOW_HEIGHT: f32 = 600.0;

const MAX_WINDOW_DIMENSION: f32 = 16_384.0;
const MAX_ABSOLUTE_ORIGIN: f32 = 100_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    maximized: bool,
}

impl WindowPlacement {
    pub fn capture(window: &Window) -> Self {
        let state = window.window_bounds();
        let bounds = state.get_bounds();
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
            maximized: matches!(state, WindowBounds::Maximized(_)) || window.is_maximized(),
        }
    }

    pub fn restore(&self, cx: &App) -> Option<WindowBounds> {
        let bounds = self.validated_bounds()?;
        let displays = cx.displays();
        if !displays.is_empty()
            && !displays
                .iter()
                .any(|display| bounds.intersects(&display.bounds()))
        {
            return None;
        }
        Some(if self.maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        })
    }

    fn validated_bounds(&self) -> Option<Bounds<Pixels>> {
        let values = [self.x, self.y, self.width, self.height];
        if values.iter().any(|value| !value.is_finite())
            || self.x.abs() > MAX_ABSOLUTE_ORIGIN
            || self.y.abs() > MAX_ABSOLUTE_ORIGIN
            || self.width <= 0.0
            || self.height <= 0.0
        {
            return None;
        }
        Some(Bounds::new(
            point(px(self.x), px(self.y)),
            size(
                px(self.width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_DIMENSION)),
                px(self.height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_DIMENSION)),
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_corrupt_window_placements_and_clamps_small_windows() {
        let invalid = WindowPlacement {
            x: f32::NAN,
            y: 0.0,
            width: 1_180.0,
            height: 760.0,
            maximized: false,
        };
        assert!(invalid.validated_bounds().is_none());

        let small = WindowPlacement {
            x: 12.0,
            y: 24.0,
            width: 200.0,
            height: 100.0,
            maximized: true,
        };
        let bounds = small.validated_bounds().expect("valid bounds");
        assert!((f32::from(bounds.size.width) - MIN_WINDOW_WIDTH).abs() < f32::EPSILON);
        assert!((f32::from(bounds.size.height) - MIN_WINDOW_HEIGHT).abs() < f32::EPSILON);
    }
}
