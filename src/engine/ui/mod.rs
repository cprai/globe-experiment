//! UI module: the panel/instrument data model and the egui [`control_panel`]
//! that renders it. Panels are laid out by taffy flexbox (via `egui_taffy`):
//! a content-sized flex column of rows - no pixel positions or fixed boxes.

mod instruments;
pub mod py;
// Constructed only by the headless binary's tree (its --scene `ui` overlay).
#[allow(dead_code)]
mod spec;
mod theme;

// The full instrument set is the reusable library surface; some instruments
// have no live producer yet, so keep the complete API exported warning-free.
#[allow(unused_imports)]
pub use instruments::{
    Button, DualReadout, Header, Instrument, InteractiveButton, InteractiveHoldButton,
    InteractiveSlider, InteractiveToggle, Lamp, LampStatus, Readout, Slider, Toggle,
};
#[allow(unused_imports)]
pub use spec::{PanelSet, UiElement, UiPanel};
pub use theme::install_theme;
use theme::{paint_bevel, paint_rivets, panel_frame};

/// Screen corner a [`UIDrawablePanel`] anchors to (mapped to an
/// `egui::Align2` in [`control_panel`]). Add more anchors when needed.
#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[pyo3::pyclass(eq, from_py_object)]
pub enum PanelAnchor {
    TopLeft,
    TopRight,
    BottomCenter,
}

/// One anchored group of instruments for a frame, generic over the scene type
/// `S` its callbacks drive. Fully owned (`'static` - it never borrows the
/// scene): a callback receives the scene as its `&mut S` argument at fire
/// time instead of capturing it; captures are limited to owned build-time
/// snapshots. Taffy computes all sizing from `rows`.
pub struct UIDrawablePanel<S> {
    pub anchor: PanelAnchor,
    /// Top-to-bottom rows, each left-to-right instruments.
    pub rows: Vec<Vec<Box<dyn Instrument<S>>>>,
}

/// Yields the frame's anchored panels. `&mut self` so readout snapshots can
/// re-propagate scene state on the spot.
///
/// **Call once per frame, before the egui `run_ui`**: egui's discard pass
/// re-runs the closure, and rebuilding panels there would refresh the
/// callback snapshots mid-frame and break their idempotency.
pub trait UIDrawable: Sized {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>>;
}

/// egui's anchor offset is measured from the corner toward the screen
/// interior, so the inset is negated on the right/bottom edges.
fn anchor_to_egui(anchor: &PanelAnchor) -> (egui::Align2, egui::Vec2) {
    let inset = theme::PANEL_INSET;
    match anchor {
        PanelAnchor::TopLeft => (egui::Align2::LEFT_TOP, egui::vec2(inset, inset)),
        PanelAnchor::TopRight => (egui::Align2::RIGHT_TOP, egui::vec2(-inset, inset)),
        PanelAnchor::BottomCenter => (egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -inset)),
    }
}

/// Renders the pre-built panels: frames each at its anchored corner, lays out
/// its rows with taffy, and threads `scene` into whichever callback fires.
///
/// `panels` must be built by [`UIDrawable::get_drawables`] ONCE per frame,
/// *outside* the egui `run_ui` closure this runs in: the discard pass
/// (`max_passes = 2`) re-runs the closure, so a click can fire its callback
/// twice in one frame - rendering the same panel objects both passes keeps
/// the build-time snapshots fixed, which is what makes the double fire
/// idempotent. Panels are therefore iterated by `&mut`, not consumed.
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
                // Extruded-metal bevel + corner screws: the bolted-down look
                // of the real Apollo panels.
                paint_bevel(ui.painter(), framed.response.rect);
                paint_rivets(ui.painter(), framed.response.rect);
            });
    }
}
