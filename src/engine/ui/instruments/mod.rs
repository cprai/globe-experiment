//! Pre-styled instruments - one self-contained type per file, each a specific
//! display with a baked-in look.
//!
//! A producer (a scene) picks *which* instrument and
//! supplies its content, but **never** its color, font, emphasis, or metrics -
//! that all lives in each instrument's [`Instrument::render`], which pulls
//! from [`crate::engine::ui::theme`] (palette + the spacing/type/radius
//! tokens). Layout is taffy flexbox: each instrument adds its own node (with
//! its own flex style - e.g. keys grow to share a row, readouts stay
//! content-sized) into the row the panel scoped for it; there are no pixel
//! positions.
//!
//! Each control is **two types**: a bare struct holding only the render data
//! ([`Button`], [`Toggle`], [`Slider`]) and an `Interactive*` wrapper that owns
//! the bare struct plus a moved `FnMut` callback ([`InteractiveButton`],
//! [`InteractiveToggle`], [`InteractiveSlider`]). The shared widget draw lives
//! on the bare struct, so both render identically; the bare struct on its own
//! is inert (clickable/draggable but does nothing). Splitting the callback out
//! lets the bare structs derive `Deserialize`, so the headless `--scene` `ui`
//! JSON deserializes straight into them (via the `ui::spec` tagged enum) with
//! no mirror type. The wrapper's borrow `'a` is the `&mut self` of the
//! producing [`crate::engine::ui::UIDrawable::get_drawables`]; each callback
//! captures a *disjoint* field of live state (e.g. one mutates `Clock::paused`,
//! another `Clock::multiplier`), so several coexist without interior
//! mutability.

mod button;
mod dual_readout;
mod header;
mod lamp;
mod readout;
mod slider;
mod toggle;

pub use button::{Button, InteractiveButton, InteractiveHoldButton};
pub use dual_readout::DualReadout;
pub use header::Header;
pub use lamp::{Lamp, LampStatus};
pub use readout::Readout;
pub use slider::{InteractiveSlider, Slider};
pub use toggle::{InteractiveToggle, Toggle};

use egui_taffy::{Tui, TuiBuilderLogic, taffy};

/// One pre-styled instrument for a frame. [`crate::engine::ui::control_panel`]
/// scopes a taffy row per panel row and calls [`render`](Instrument::render)
/// for each instrument in it; the instrument adds its own flex node(s) into
/// that row.
///
/// `render` takes `&mut self` so a control can fire its `FnMut` callback;
/// instruments are consumed for the frame (the panel's rows are drained each
/// frame).
pub trait Instrument {
    /// Adds this instrument into its row's `tui`. The instrument owns its
    /// node's flex style (grow/size) as part of its baked-in look; the row and
    /// panel styles come from [`crate::engine::ui::theme`].
    fn render(&mut self, tui: &mut Tui);
}

/// Adds one egui leaf node with the given taffy `style` and runs `content`
/// inside it. Shared scope setup for every instrument that draws plain egui
/// widgets: top-down layout and wrapping disabled (an auto-wrapping label
/// can't grow its node back after a shorter label shrank it, so a Play/Pause
/// key would ratchet smaller).
pub(super) fn leaf<T>(
    tui: &mut Tui,
    style: taffy::Style,
    content: impl FnOnce(&mut egui::Ui) -> T,
) -> T {
    tui.style(style)
        .egui_layout(egui::Layout::top_down(egui::Align::Min))
        .wrap_mode(egui::TextWrapMode::Extend)
        .ui(content)
}
