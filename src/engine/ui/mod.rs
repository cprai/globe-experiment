//! UI module: the panel/instrument data model and the egui `control_panel` that
//! renders it.
//!
//! Layout:
//! - [`instruments`] - one self-contained instrument type per file (each impls
//!   [`Instrument`]); style lives in each instrument's `render`.
//! - [`theme`] - the Apollo-panel palette, the spacing/type/radius tokens, the
//!   taffy panel/row styles, [`install_theme`], and panel chrome.
//! - [`spec`] - the serde-deserialized [`PanelSet`]/[`UiPanel`]/[`UiElement`]
//!   for the headless `--scene` overlay.
//! - this file - the [`UIDrawable`] trait, [`UIDrawablePanel`]/[`PanelAnchor`],
//!   and [`control_panel`]. Each scene implements `UIDrawable` itself, building
//!   its own Time panel plus any scene panels.
//!
//! Panels are laid out by taffy flexbox (via `egui_taffy`): a panel is a
//! content-sized flex column of rows, each row a flex row of instruments -
//! there are no absolute pixel positions or fixed panel boxes. A producer
//! groups instruments into rows; every metric comes from the `theme` tokens.
//!
//! The clock lives directly in each scene struct (reached through the
//! scenes' SceneClock API). The
//! panel reads/drives a scene through its `UIDrawable` impl, which is kept
//! separate from the `Scene` trait.

mod instruments;
// The Python face of this API (the `Panel` pyclass, the `Interactive*`
// script twins, and the Python->Rust panel conversion). Public: the `*_py`
// scenes convert their script's return through it, and `engine::py` registers
// its classes into the embedded `globe` module. Only the main binary's tree
// constructs it (the headless bin runs no Python; its crate-level
// `allow(dead_code)` covers the module there).
pub mod py;
// The spec types are constructed only by the headless binary's tree (its
// `--scene` `ui` overlay deserializes into them); in the main binary's tree
// they are intentionally unconstructed.
#[allow(dead_code)]
mod spec;
mod theme;

// The full instrument set is re-exported as the reusable UI library's surface.
// Some instruments (e.g. `Button`/`Lamp`/`InteractiveButton`) have no live
// producer yet, so this binary never names them - `allow(unused_imports)` keeps
// the complete library API exported without a warning until a producer uses it.
#[allow(unused_imports)]
pub use instruments::{
    Button, DualReadout, Header, Instrument, InteractiveButton, InteractiveHoldButton,
    InteractiveSlider, InteractiveToggle, Lamp, LampStatus, Readout, Slider, Toggle,
};
#[allow(unused_imports)]
pub use spec::{PanelSet, UiElement, UiPanel};
pub use theme::install_theme;
use theme::{paint_bevel, paint_rivets, panel_frame};

/// Which screen edge point a [`UIDrawablePanel`] anchors to. egui-free (mapped
/// to an `egui::Align2` in [`control_panel`]); anchoring keeps a panel pinned
/// to its spot as the window resizes. Only the anchors currently in use are
/// listed - add the bottom corners when a panel needs one.
///
/// `Copy` so a [`PanelSet`] can hand it out of a borrowing `get_drawables` by
/// value; `Deserialize` so the headless `--scene` `ui` JSON can name an anchor
/// (`"top_left"` / `"top_right"` / `"bottom_center"`); `pyclass` so a scene
/// script can name one (`PanelAnchor.TopLeft`, with `eq` for comparisons) -
/// the dual Rust/Python UI API (see `engine::py`).
#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[pyo3::pyclass(module = "globe", eq, from_py_object)]
pub enum PanelAnchor {
    TopLeft,
    TopRight,
    BottomCenter,
}

/// One anchored group of UI instruments for a frame, generic over the scene
/// type `S` its interactive callbacks drive. The panel owns only its corner
/// `anchor` (inset by the shared `theme::PANEL_INSET`); everything else
/// (its size and every instrument's place) is computed by taffy from `rows`:
/// a flex column of flex rows, sized to content with the shared minimum width.
///
/// The panel is fully owned (`'static` - it never borrows the scene): a
/// control's callback receives the scene as its `&mut S` argument when it
/// fires (threaded in by [`control_panel`]) instead of capturing it, which is
/// what lets every callback coexist AND call `&mut self` scene APIs (e.g.
/// the `SceneClock` setters) directly. Captured state is limited to owned
/// build-time snapshots.
pub struct UIDrawablePanel<S> {
    pub anchor: PanelAnchor,
    /// Top-to-bottom rows, each left-to-right instruments.
    pub rows: Vec<Vec<Box<dyn Instrument<S>>>>,
}

/// Anything the control panel can render: it yields a list of anchored
/// [`UIDrawablePanel`]s, each owning rows of [`Instrument`]s. Implemented by
/// each scene (which returns its Time panel plus its own scene panels).
/// `&mut self` so readout snapshots can re-propagate scene state on the spot.
///
/// **Call once per frame, before the egui `run_ui`** (then render the result
/// with [`control_panel`] inside it). egui's discard pass re-runs the
/// `run_ui` closure; rebuilding the panels there would refresh the callback
/// snapshots mid-frame and break their idempotency (a re-fired Run toggle
/// would flip the clock twice, losing the click).
pub trait UIDrawable: Sized {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>>;
}

/// Maps a [`PanelAnchor`] to egui's `Area::anchor` arguments, inset by the
/// shared `theme::PANEL_INSET`. egui's offset is measured from the anchored
/// corner toward the screen interior, so the inset is negated on the
/// right/bottom edges.
fn anchor_to_egui(anchor: &PanelAnchor) -> (egui::Align2, egui::Vec2) {
    let inset = theme::PANEL_INSET;
    match anchor {
        PanelAnchor::TopLeft => (egui::Align2::LEFT_TOP, egui::vec2(inset, inset)),
        PanelAnchor::TopRight => (egui::Align2::RIGHT_TOP, egui::vec2(-inset, inset)),
        PanelAnchor::BottomCenter => (egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -inset)),
    }
}

/// The control/readout panel(s) over the scene: one panel holds the simulation
/// clock (datetime, play/pause, and speed); a scene may add its own (e.g.
/// per-satellite position readouts, or a camera-target selector).
///
/// This function is deliberately *decoupled from interactivity*: it knows
/// nothing about the `Clock` or any scene type. It frames each pre-built
/// panel at its anchored corner and lays out its rows with taffy
/// (`theme::panel_layout` / `theme::row_layout`); each instrument adds its
/// own flex node. Interactivity rides along inside each instrument: an
/// interactive control wraps its bare struct with a callback that receives
/// `scene` when it fires, while a bare (e.g. deserialized) control renders
/// but does nothing - which is what lets the same code render a mock panel.
///
/// `panels` must be built by [`UIDrawable::get_drawables`] ONCE per frame,
/// *outside* the egui `run_ui` closure this runs in: egui's discard pass
/// (`install_theme` sets `max_passes = 2`, which egui_taffy needs to settle
/// its layout same-frame) re-runs the closure with the same input, so a
/// click can fire its callback twice in one frame. Rendering the same panel
/// objects both passes keeps the callbacks' build-time snapshots fixed, so
/// the double fire is idempotent; panels are therefore iterated by `&mut`,
/// not consumed.
pub fn control_panel<S>(ctx: &egui::Context, panels: &mut [UIDrawablePanel<S>], scene: &mut S) {
    use egui_taffy::TuiBuilderLogic;

    for (panel_index, panel) in panels.iter_mut().enumerate() {
        let (align, offset) = anchor_to_egui(&panel.anchor);
        egui::Area::new(egui::Id::new(("ui_panel", panel_index)))
            .anchor(align, offset)
            .show(ctx, |ui| {
                let framed = panel_frame().show(ui, |ui| {
                    egui_taffy::tui(ui, ui.id().with("layout"))
                        .style(theme::panel_layout())
                        .show(|tui| {
                            for row in &mut panel.rows {
                                tui.style(theme::row_layout()).add(|tui| {
                                    for element in row.iter_mut() {
                                        element.render(tui, &mut *scene);
                                    }
                                });
                            }
                        });
                });
                // Fake an extruded-metal bevel along the top and left, then
                // slotted screws in the corners - the raised, bolted-down look
                // of the real Apollo panels.
                paint_bevel(ui.painter(), framed.response.rect);
                paint_rivets(ui.painter(), framed.response.rect);
            });
    }
}
