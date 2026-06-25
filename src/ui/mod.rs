//! UI module: the panel/instrument data model and the egui `control_panel` that
//! renders it.
//!
//! Layout:
//! - [`instruments`] - one self-contained instrument type per file (each impls
//!   [`Instrument`]); style lives in each instrument's `render`.
//! - [`theme`] - the Apollo-panel palette, [`install_theme`], and panel chrome.
//! - [`mock`] - the serde-derived [`MockUi`]/[`UiPanelSpec`] for the `render
//!   --scene` overlay.
//! - this file - the [`UIDrawable`] trait, [`UIDrawablePanel`]/[`PanelAnchor`],
//!   the live `impl UIDrawable for SimulationState`, and [`control_panel`].
//!
//! `SimulationState` (clock + celestial sphere) is the shared core that every
//! scenario struct holds by composition. The panel reads/drives a scenario
//! through this `UIDrawable` impl (separate from the `Simulation` trait), so
//! `simulation` has no UI dependency.

mod instruments;
mod mock;
mod theme;

use crate::simulation::{Clock, SimulationState};
pub use instruments::{DualReadout, Header, Instrument, Readout, Slider, Toggle};
pub use mock::{MockUi, UiPanelSpec};
pub use theme::install_theme;
use theme::{paint_bevel, paint_rivets, panel_frame};

/// Which screen corner a [`UIDrawablePanel`] anchors to. egui-free (mapped to
/// an `egui::Align2` in [`control_panel`]); anchoring keeps a panel pinned to
/// its corner as the window resizes. Only the corners currently in use are
/// listed - add the bottom corners when a panel needs one.
///
/// `Copy` so a [`MockUi`] can hand it out of a borrowing `get_drawables` by
/// value; `Deserialize` so the `render --scene` `ui` JSON can name a corner
/// (`"top_left"` / `"top_right"`).
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelAnchor {
    TopLeft,
    TopRight,
}

/// One positioned group of UI instruments for a frame. The panel owns its place
/// on screen (a corner `anchor` plus an inset `offset`, both resolved against
/// the live window in [`control_panel`]); its `elements` carry positions
/// *relative* to the panel's content origin. `size` fixes the panel's box - it
/// both sizes the frame and pins the egui `Area` size so it can't auto-shrink
/// frame to frame.
///
/// `elements` are boxed [`Instrument`] trait objects; their borrow `'a` is the
/// `&mut self` of the producing [`UIDrawable::get_drawables`], so a control's
/// callback can capture a disjoint mutable field of live state.
pub struct UIDrawablePanel<'a> {
    pub anchor: PanelAnchor,
    /// Inset (egui points) from the anchored corner, toward the screen
    /// interior.
    pub offset: [f32; 2],
    /// Panel box size (egui points).
    pub size: [f32; 2],
    pub elements: Vec<Box<dyn Instrument + 'a>>,
}

/// Anything the control panel can render: it yields a list of positioned
/// [`UIDrawablePanel`]s, each owning a group of relatively-placed
/// [`Instrument`]s. Implemented by [`SimulationState`] (one shared-core panel)
/// and by each scenario (which returns the core panel plus its own
/// per-satellite panel). `&mut self` so a control's callback can capture a
/// disjoint mutable field of live state.
pub trait UIDrawable {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>>;
}

impl UIDrawable for SimulationState {
    /// The shared-core panel, read from live state: the ephemeris subsolar
    /// point, the clock datetime, and the play/pause + speed controls whose
    /// callbacks mutate the live clock. The two control callbacks capture
    /// disjoint clock fields (`paused` vs `multiplier`) via direct field
    /// assignment - a `Clock` method would borrow the whole clock and collide.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // Snapshot the displayed values up front (owned `String`/`f32`/`bool`),
        // so no shared borrow of the clock outlives into the mutable callback
        // captures below.
        let datetime = self.clock.datetime_label();
        let subsolar_lat = format!("{:.2} deg", self.celestial_sphere.subsolar_lat_deg);
        let subsolar_lon = format!("{:.2} deg", self.celestial_sphere.subsolar_lon_deg);
        let speed = format!("{:.1}x", self.clock.multiplier);
        let running = !self.clock.paused;

        // Exponential (base e) speed: the slider edits the exponent, so
        // multiplier = e^exp - real time (e^0 = 1x) at the left, 100x at the
        // right, 10x at the midpoint. The mapping lives here, not in the panel.
        let speed_exp = self.clock.multiplier.ln();
        let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();

        // Instrument positions are relative to this panel's content origin. The
        // producer picks instruments + content only; all styling is in the
        // instrument modules.
        let elements: Vec<Box<dyn Instrument + '_>> = vec![
            Box::new(Header {
                position: [0.0, 0.0],
                title: "Time / Subsolar".to_string(),
            }),
            Box::new(Readout {
                position: [0.0, 26.0],
                label: "UTC".to_string(),
                value: datetime,
            }),
            Box::new(DualReadout {
                position: [0.0, 52.0],
                left_label: "Lat".to_string(),
                left_value: subsolar_lat,
                right_label: "Lon".to_string(),
                right_value: subsolar_lon,
            }),
            Box::new(Toggle {
                position: [0.0, 84.0],
                label: "Run".to_string(),
                active: running,
                on_toggle: Some(Box::new(|| self.clock.paused = !self.clock.paused)),
            }),
            Box::new(Readout {
                position: [104.0, 86.0],
                label: "Speed".to_string(),
                value: speed,
            }),
            Box::new(Slider {
                position: [0.0, 114.0],
                value: speed_exp,
                range: exp_range,
                on_change: Some(Box::new(|exp| self.clock.multiplier = exp.exp())),
            }),
        ];

        vec![UIDrawablePanel {
            anchor: PanelAnchor::TopLeft,
            offset: [10.0, 10.0],
            size: [340.0, 148.0],
            elements,
        }]
    }
}

/// Maps a [`PanelAnchor`] + inset to egui's `Area::anchor` arguments. egui's
/// offset is measured from the anchored corner toward the screen interior, so
/// the inset is negated on the right/bottom edges.
fn anchor_to_egui(anchor: &PanelAnchor, offset: [f32; 2]) -> (egui::Align2, egui::Vec2) {
    let [x, y] = offset;
    match anchor {
        PanelAnchor::TopLeft => (egui::Align2::LEFT_TOP, egui::vec2(x, y)),
        PanelAnchor::TopRight => (egui::Align2::RIGHT_TOP, egui::vec2(-x, y)),
    }
}

/// The control/readout panel(s) over the globe: one panel holds the simulation
/// clock (play/pause and speed) plus the ephemeris-driven subsolar point;
/// another holds each tracked station's datetime and position.
///
/// This function is deliberately *decoupled from interactivity*: it knows
/// nothing about the `Clock` or any scenario. It asks the `drawable` for a list
/// of [`UIDrawablePanel`]s, frames each at its anchored position, and renders
/// each panel's [`Instrument`]s at their panel-relative positions.
/// Interactivity rides along as optional callbacks; a control whose callback is
/// `None` renders but does nothing - which is what lets the same code render a
/// mock panel.
pub fn control_panel(ctx: &egui::Context, drawable: &mut impl UIDrawable) {
    for (panel_index, panel) in drawable.get_drawables().into_iter().enumerate() {
        let (align, offset) = anchor_to_egui(&panel.anchor, panel.offset);
        let size = egui::vec2(panel.size[0], panel.size[1]);
        egui::Area::new(egui::Id::new(("ui_panel", panel_index)))
            .anchor(align, offset)
            .show(ctx, |ui| {
                let framed = panel_frame().show(ui, |ui| {
                    // Fix the box to the requested size: it makes the frame a
                    // consistent rectangle and pins the Area size so it can't
                    // auto-shrink against the previous frame's content.
                    ui.set_min_size(size);
                    let origin = ui.min_rect().min;
                    for mut element in panel.elements {
                        render_element(ui, origin, size, element.as_mut());
                    }
                });
                // Fake an extruded-metal bevel along the top and left, then
                // slotted screws in the corners - the raised, bolted-down look
                // of the real Apollo panels.
                paint_bevel(ui.painter(), framed.response.rect);
                paint_rivets(ui.painter(), framed.response.rect);
            });
    }
}

/// Renders one instrument inside a panel at its panel-relative position. Each
/// instrument gets its own child `Ui` anchored at `origin + position`,
/// extending to the panel's bottom-right so the widget lays out top-left from
/// there. Wrapping is disabled everywhere: an auto-wrapping label can't grow
/// its area back after a shorter label shrank it, so a Play/Pause toggle would
/// ratchet smaller. The common scope/wrap setup lives here; the per-instrument
/// look lives in each [`Instrument::render`].
fn render_element(
    ui: &mut egui::Ui,
    origin: egui::Pos2,
    size: egui::Vec2,
    element: &mut dyn Instrument,
) {
    let position = element.position();
    let child_rect =
        egui::Rect::from_min_max(origin + egui::vec2(position[0], position[1]), origin + size);
    let builder = egui::UiBuilder::new()
        .max_rect(child_rect)
        .layout(egui::Layout::top_down(egui::Align::Min));
    ui.scope_builder(builder, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        element.render(ui, child_rect, size);
    });
}
