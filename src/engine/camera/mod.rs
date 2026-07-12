//! The [`CameraControl`] + [`CameraView`] trait pair, the winit-free input
//! vocabulary, and the reusable [`PtzCamera`] (embedded via
//! [`ScenePtzCamera`]). Winit-free on purpose: both bin trees build this
//! module, and device-neutral input types keep a future gamepad/touch scheme
//! a new defaulted method plus one `application` translation arm.

mod ptz;

pub use ptz::PtzCamera;
// Named only by the main tree's scenes; the crate-level allow on the
// headless root covers dead code, not unused imports.
#[allow(unused_imports)]
pub use ptz::ScenePtzCamera;
// The derive macro shares the trait's name (macro vs type namespace), so
// one `use engine::camera::ScenePtzCamera` imports both - the serde pattern.
#[allow(unused_imports)]
pub use macros::ScenePtzCamera;

use crate::engine::scene::RenderState;

/// Which pointer button an event names, device-neutral.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerButton {
    Left,
    Right,
}

/// One scroll event's magnitude. Both variants are load-bearing: discrete
/// wheels (Windows/X11) deliver lines, precision trackpads (macOS) pixels;
/// the zoom feel is tuned per variant.
#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    /// Wheel notches.
    Lines(f64),
    /// Precision-device pixels.
    Pixels(f64),
}

/// What the scene cursor should look like, winit-free.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorHint {
    Default,
    Grab,
    Grabbing,
}

/// The interactive-input half of the camera interface. Every method defaults
/// to a no-op, so a non-interactive camera implements only [`CameraView`].
/// No return values: the app renders every frame regardless, so there is no
/// "camera changed, redraw" signal to carry.
pub trait CameraControl {
    /// A pointer button went down. Carries no position (winit press events
    /// have none); a camera uses the position last given to
    /// [`pointer_move`](Self::pointer_move).
    fn pointer_press(&mut self, _button: PointerButton) {}

    /// A pointer button was released.
    fn pointer_release(&mut self, _button: PointerButton) {}

    /// The pointer moved to `position` (physical pixels, window-relative).
    /// `viewport_height` (pixels) scales drag gestures to the view.
    fn pointer_move(&mut self, _position: (f64, f64), _viewport_height: f64) {}

    /// A scroll wheel / trackpad event.
    fn scroll(&mut self, _delta: ScrollDelta) {}

    /// Advances one frame of camera animation (flick coast, zoom glide) with
    /// real frame time. Called at the top of every redraw.
    fn tick(&mut self, _viewport_height: f64) {}

    /// The scene cursor to show while the pointer is not over an egui panel.
    fn cursor_hint(&self) -> CursorHint {
        CursorHint::Default
    }
}

/// The frame-production half of the camera interface, split from
/// [`CameraControl`] so a scripted or fixed camera needs no input surface.
pub trait CameraView {
    /// Produce this frame's [`RenderState`]: resolve the scene-owned camera
    /// target and the rig against `CelestialSphere::at` evaluated at the
    /// frame's clock instant, and pack it with the time and markers.
    fn frame_state(&mut self) -> RenderState;
}
