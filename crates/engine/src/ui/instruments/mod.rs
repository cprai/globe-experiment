//! Pre-styled instruments, one self-contained type per file. Style lives in
//! each [`Instrument::render`] (pulling from `theme`); producers pick which
//! instrument and its content only. Each control is a bare data struct
//! (derives `Deserialize`; inert on its own) plus an `Interactive*` wrapper
//! holding a moved `FnMut(&mut S)` callback. Callbacks never capture the
//! scene - they receive it as `&mut S` at fire time - and **must be
//! idempotent** (write-only or snapshot-based): egui's `max_passes = 2`
//! discard pass can fire the same callback twice in one frame.

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

/// One pre-styled instrument for a frame, generic over the scene type `S` it
/// can drive. `render` adds the instrument's own flex node(s) - it owns that
/// node's style - into the row `control_panel` scoped; the scene argument
/// exists so an interactive control can hand it to its callback (bare
/// instruments ignore it).
pub trait Instrument<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S);
}

/// An interactive control's callback, fired with the live scene.
pub type Callback<S> = Box<dyn FnMut(&mut S)>;

/// [`Callback`] carrying the new value - the slider's edit callback.
pub type ValueCallback<S> = Box<dyn FnMut(&mut S, f32)>;

/// Adds one egui leaf node with the given taffy `style` and runs `content`
/// inside it: top-down layout, wrapping disabled (an auto-wrapping label
/// cannot grow its node back after a shorter label shrank it, so e.g. a
/// Play/Pause key would ratchet smaller).
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
