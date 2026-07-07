//! The camera layer: the [`CameraControl`] + [`CameraView`] trait pair every
//! scene implements (alongside `Scene` + `UIDrawable`), the winit-free
//! input vocabulary they speak, and the reusable [`PtzCamera`] pan/tilt/zoom
//! implementation a scene can embed - or not: a future scene may fly a
//! scripted, fixed, or chase camera by implementing the traits differently
//! (a non-interactive camera implements only `CameraView` and leaves every
//! `CameraControl` method at its no-op default).
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

use crate::engine::scene::RenderState;

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

/// The interactive-input half of the camera interface: responding to
/// (already-translated) window input, advancing the animation those gestures
/// spawn (flick coasting, the zoom glide), and reporting the cursor
/// affordance that reflects the drag state. The methods return whether the
/// camera changed (or an animation started/continues) so the application
/// knows a redraw is needed; every method defaults to a no-op, so a scene
/// with a non-interactive camera implements only [`CameraView`].
pub trait CameraControl {
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
    /// `Scene::advance`; returns true while another frame is needed, so
    /// a settled camera lets the app go idle.
    fn tick(&mut self, _viewport_height: f64) -> bool {
        false
    }

    /// The scene cursor to show while the pointer is not over an egui panel.
    fn cursor_hint(&self) -> CursorHint {
        CursorHint::Default
    }
}

/// The frame-production half of the camera interface: turning the scene's
/// own simulation state into the frame's [`RenderState`]. Split from
/// [`CameraControl`] so the two concerns stay independent - a scripted or
/// fixed camera is a `CameraView` with no input surface at all.
pub trait CameraView {
    /// Produce this frame's [`RenderState`]: resolve the frame's camera
    /// target (owned by the scene; a genuine body switch reframes the
    /// camera), resolve the rig against the scene's own celestial sphere,
    /// and pack it with the frame's time and markers. The marker propagation
    /// happens here; the immediately-following `UIDrawable::get_drawables`
    /// call (the egui panel) re-derives its readouts at the same clock
    /// instant, so they match the rendered markers.
    fn frame_state(&mut self) -> RenderState;
}
