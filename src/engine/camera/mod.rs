//! The camera layer: the [`Camera`] trait every scenario implements
//! (alongside `Simulation` + `UIDrawable`), the winit-free input vocabulary
//! it speaks, and the reusable [`PtzCamera`] pan/tilt/zoom implementation a
//! scenario can embed - or not: a future scenario may fly a scripted, fixed,
//! or chase camera by implementing the trait differently.
//!
//! Winit-free on purpose: both bin trees build this module (the headless
//! binary constructs a `PtzCamera` straight from its `--scene` JSON), and the
//! input types are device-neutral, so a future gamepad or touch scheme is a
//! new defaulted trait method plus an `application`-side translation, not a
//! camera rewrite. The application keeps NO input state - it translates each
//! winit event into one call here statelessly (cursor tracking lives in the
//! camera).

mod ptz;

pub use ptz::PtzCamera;

use crate::engine::simulation::RenderState;

/// Which pointer button an event names, device-neutral. The application's
/// winit translation maps the left/right mouse buttons here and drops the
/// rest, so a camera never sees a button it has no gesture for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PointerButton {
    Left,
    Right,
}

/// One scroll event's magnitude. Both variants are load-bearing: discrete
/// wheels (Windows/X11) deliver lines while precision trackpads (macOS)
/// deliver pixels, and the zoom feel is tuned per variant - dropping either
/// kills scroll-zoom on half the platform matrix.
#[derive(Clone, Copy, Debug)]
pub enum ScrollDelta {
    /// Wheel notches (winit `LineDelta`'s vertical component).
    Lines(f64),
    /// Precision-device pixels (winit `PixelDelta`'s vertical component).
    Pixels(f64),
}

/// What the scene cursor should look like, winit-free (the application maps
/// it onto the winit icon set).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CursorHint {
    /// No camera interaction affordance (a non-interactive camera).
    Default,
    /// The scene can be grabbed (dragged).
    Grab,
    /// A drag is in progress.
    Grabbing,
}

/// The camera interface every scenario implements. It owns everything
/// view-related for its scenario: responding to (already-translated) window
/// input, advancing camera animation, and producing the frame's
/// [`RenderState`] from the scenario's own simulation state. The input
/// methods return whether the camera changed (or an animation started) so
/// the application knows a redraw is needed; their defaults are no-ops, so a
/// scenario with a non-interactive camera implements only `frame_state`.
pub trait Camera {
    /// A pointer button went down. Carries no position: winit press events
    /// have none, so a camera uses the position last given to
    /// [`pointer_move`](Self::pointer_move) (the cursor tracking state lives
    /// in the camera, not the application).
    fn pointer_press(&mut self, _button: PointerButton) -> bool {
        false
    }

    /// A pointer button was released.
    fn pointer_release(&mut self, _button: PointerButton) -> bool {
        false
    }

    /// The pointer moved to `position` (physical pixels, window-relative).
    /// `viewport_height` (pixels) scales drag gestures to the view.
    fn pointer_move(&mut self, _position: (f64, f64), _viewport_height: f64) -> bool {
        false
    }

    /// A scroll wheel / trackpad event.
    fn scroll(&mut self, _delta: ScrollDelta) -> bool {
        false
    }

    /// Advances one frame of camera animation (e.g. flick coasting, the zoom
    /// glide) with real frame time. Called at the top of every redraw, before
    /// `Simulation::advance`; returns true while another frame is needed, so
    /// a settled camera lets the app go idle.
    fn tick(&mut self, _viewport_height: f64) -> bool {
        false
    }

    /// The scene cursor to show while the pointer is not over an egui panel.
    fn cursor_hint(&self) -> CursorHint {
        CursorHint::Default
    }

    /// Produce this frame's [`RenderState`]: re-aim the camera at the frame's
    /// target, resolve the rig against the scenario's own celestial sphere,
    /// and pack it with the frame's time and markers. Satellite propagation
    /// happens here, once per frame per satellite; the per-satellite readout
    /// from the same propagation is stashed on the scenario so the
    /// immediately-following `UIDrawable::get_drawables` call (the egui
    /// panel) reports values matching the rendered markers.
    fn frame_state(&mut self) -> RenderState;
}
