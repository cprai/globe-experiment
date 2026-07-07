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
//! The clock + celestial sphere live directly in each scene struct. The
//! panel reads/drives a scene through its `UIDrawable` impl, which is kept
//! separate from the `Scene` trait.

mod instruments;
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
/// (`"top_left"` / `"top_right"` / `"bottom_center"`).
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelAnchor {
    TopLeft,
    TopRight,
    BottomCenter,
}

/// One anchored group of UI instruments for a frame. The panel owns only its
/// corner `anchor` (inset by the shared `theme::PANEL_INSET`); everything else
/// (its size and every instrument's place) is computed by taffy from `rows`:
/// a flex column of flex rows, sized to content with the shared minimum width.
///
/// The boxed [`Instrument`] trait objects' borrow `'a` is the `&mut self` of
/// the producing [`UIDrawable::get_drawables`], so a control's callback can
/// capture a disjoint mutable field of live state.
pub struct UIDrawablePanel<'a> {
    pub anchor: PanelAnchor,
    /// Top-to-bottom rows, each left-to-right instruments.
    pub rows: Vec<Vec<Box<dyn Instrument + 'a>>>,
}

/// Anything the control panel can render: it yields a list of anchored
/// [`UIDrawablePanel`]s, each owning rows of [`Instrument`]s. Implemented by
/// each scene (which returns its Time panel plus its own scene panels).
/// `&mut self` so a control's callback can capture a disjoint mutable field
/// of live state.
pub trait UIDrawable {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>>;
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
/// nothing about the `Clock` or any scene. It asks the `drawable` for a list
/// of [`UIDrawablePanel`]s, frames each at its anchored corner, and lays out
/// each panel's rows with taffy (`theme::panel_layout` / `theme::row_layout`);
/// each instrument adds its own flex node. Interactivity rides along inside
/// each instrument: an interactive control wraps its bare struct with a
/// callback, while a bare (e.g. deserialized) control renders but does nothing
/// - which is what lets the same code render a mock panel.
pub fn control_panel(ctx: &egui::Context, drawable: &mut impl UIDrawable) {
    use egui_taffy::TuiBuilderLogic;

    for (panel_index, panel) in drawable.get_drawables().into_iter().enumerate() {
        let (align, offset) = anchor_to_egui(&panel.anchor);
        egui::Area::new(egui::Id::new(("ui_panel", panel_index)))
            .anchor(align, offset)
            .show(ctx, |ui| {
                let framed = panel_frame().show(ui, |ui| {
                    egui_taffy::tui(ui, ui.id().with("layout"))
                        .style(theme::panel_layout())
                        .show(|tui| {
                            for row in panel.rows {
                                tui.style(theme::row_layout()).add(|tui| {
                                    for mut element in row {
                                        element.render(tui);
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
