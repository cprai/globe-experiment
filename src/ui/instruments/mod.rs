//! Pre-styled instruments - one self-contained type per file, each a specific
//! display with a baked-in look.
//!
//! A producer (a scenario / `SimulationState`) picks *which* instrument and
//! supplies its content, but **never** its color, font, or emphasis - that all
//! lives in each instrument's [`Instrument::render`], which pulls from
//! [`crate::ui::theme`].
//!
//! Each control is **two types**: a bare struct holding only the render data
//! ([`Button`], [`Toggle`], [`Slider`]) and an `Interactive*` wrapper that owns
//! the bare struct plus a moved `FnMut` callback ([`InteractiveButton`],
//! [`InteractiveToggle`], [`InteractiveSlider`]). The shared widget draw lives
//! on the bare struct, so both render identically; the bare struct on its own
//! is inert (clickable/draggable but does nothing). Splitting the callback out
//! lets the bare structs derive `Deserialize`, so the `render --scene` `ui`
//! JSON deserializes straight into them (via the `ui::spec` tagged enum) with
//! no mirror type. The wrapper's borrow `'a` is the `&mut self` of the
//! producing [`crate::ui::UIDrawable::get_drawables`]; each callback captures a
//! *disjoint* field of live state (e.g. one mutates `Clock::paused`, another
//! `Clock::multiplier`), so several coexist without interior mutability.

mod button;
mod dual_readout;
mod header;
mod lamp;
mod readout;
mod slider;
mod toggle;

pub use button::{Button, InteractiveButton};
pub use dual_readout::DualReadout;
pub use header::Header;
pub use lamp::{Lamp, LampStatus};
pub use readout::Readout;
pub use slider::{InteractiveSlider, Slider};
pub use toggle::{InteractiveToggle, Toggle};

/// One pre-styled instrument for a frame. Implemented per instrument type;
/// [`crate::ui::control_panel`] places each instrument at its panel-relative
/// [`position`](Instrument::position) and then calls
/// [`render`](Instrument::render) into the already-scoped child `Ui`.
///
/// `render` takes `&mut self` so a control can fire its `FnMut` callback;
/// instruments are consumed for the frame (the panel's element vector is
/// drained each frame).
pub trait Instrument {
    /// This instrument's top-left, **relative to its containing panel's content
    /// origin** (egui points). Resolved against the panel's on-screen origin by
    /// [`crate::ui::control_panel`].
    fn position(&self) -> [f32; 2];

    /// Renders this instrument into its already-scoped child `Ui` (anchored at
    /// the instrument's position, extending to the panel's bottom-right, with
    /// wrapping disabled). `child_rect` is that allocated box and `panel_size`
    /// the containing panel's box - only [`Header`] uses them (for its
    /// full-width rule); every other instrument ignores them.
    fn render(&mut self, ui: &mut egui::Ui, child_rect: egui::Rect, panel_size: egui::Vec2);
}
