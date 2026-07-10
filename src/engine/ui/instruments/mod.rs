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
//! the bare struct plus a moved `FnMut(&mut S)` callback
//! ([`InteractiveButton`], [`InteractiveToggle`], [`InteractiveSlider`]). The
//! shared widget draw lives on the bare struct, so both render identically; the
//! bare struct on its own is inert (clickable/draggable but does nothing).
//! Splitting the callback out lets the bare structs derive `Deserialize`, so
//! the headless `--scene` `ui` JSON deserializes straight into them (via the
//! `ui::spec` tagged enum) with no mirror type. A callback never *captures* the
//! producing scene (many coexist, so no one closure could hold `&mut self`): it
//! receives the scene as its `&mut S` argument when it fires - only one
//! callback runs at a time, so exclusive access is trivially satisfied - and
//! may call any `&mut self` scene API directly (e.g. the `SceneClock` setters).
//! Captures are limited to owned snapshots taken at panel-build time, which
//! keeps the panels `'static`. **Callbacks must be idempotent** (write-only or
//! snapshot-based, never read-modify-write): egui's `max_passes = 2` discard
//! pass can fire the same callback twice in one frame against the
//! once-per-frame-built panel.

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
/// can drive. [`crate::engine::ui::control_panel`] scopes a taffy row per
/// panel row and calls [`render`](Instrument::render) for each instrument in
/// it; the instrument adds its own flex node(s) into that row.
///
/// `render` receives the live scene so an interactive control can hand it to
/// its `FnMut(&mut S)` callback at fire time (bare instruments ignore it -
/// they impl `Instrument<S>` for every `S`). `&mut self` because the callback
/// is `FnMut`; the same panel is re-rendered on egui's discard pass, so
/// instruments are not consumed per pass.
pub trait Instrument<S> {
    /// Adds this instrument into its row's `tui`. The instrument owns its
    /// node's flex style (grow/size) as part of its baked-in look; the row and
    /// panel styles come from [`crate::engine::ui::theme`].
    fn render(&mut self, tui: &mut Tui, scene: &mut S);
}

/// An interactive control's callback: fired with the live scene (see the
/// module doc for the no-capture/idempotency rules). The key/toggle wrappers
/// all store one.
pub type Callback<S> = Box<dyn FnMut(&mut S)>;

/// [`Callback`] carrying a new value alongside the scene - the slider's edit
/// callback.
pub type ValueCallback<S> = Box<dyn FnMut(&mut S, f32)>;

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
