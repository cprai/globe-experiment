//! The headless `--scene` `ui` overlay: the panels deserialized straight from
//! the scene JSON and run through the exact same [`crate::ui::control_panel`]
//! path as the live app, so a mock layout is faithful to real output.
//!
//! Because the interactive instruments split their callback into an
//! `Interactive*` wrapper, the bare instrument structs are pure render data and
//! derive `Deserialize` themselves - so this module is just a tagged enum over
//! them ([`UiElement`]) plus a panel ([`UiPanel`]); there is no field-
//! duplicating mirror type. The deserialized controls are inert (no callback),
//! which is exactly a mock panel. Like the live model, a panel is `rows` of
//! elements - taffy computes every position and the panel size, so the JSON
//! carries no pixel coordinates.

use super::instruments::{Button, DualReadout, Header, Instrument, Lamp, Readout, Slider, Toggle};
use super::{PanelAnchor, UIDrawable, UIDrawablePanel};

/// One deserialized instrument, tagged by instrument name in snake_case, e.g.
/// `{"readout": {"label": "ALT", "value": "417", "unit": "km"}}`. Each variant
/// wraps the real (bare, callback-free) instrument struct, so the JSON shape is
/// just that struct's fields - no separate mock type to keep in sync.
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiElement {
    Header(Header),
    Readout(Readout),
    DualReadout(DualReadout),
    Button(Button),
    Toggle(Toggle),
    Lamp(Lamp),
    Slider(Slider),
}

impl UiElement {
    /// Boxes a clone of the wrapped instrument as a trait object. Cloned (not
    /// moved) because [`PanelSet::get_drawables`] runs every frame off a shared
    /// borrow of the owned spec.
    fn to_instrument(&self) -> Box<dyn Instrument> {
        match self {
            UiElement::Header(header) => Box::new(header.clone()),
            UiElement::Readout(readout) => Box::new(readout.clone()),
            UiElement::DualReadout(dual) => Box::new(dual.clone()),
            UiElement::Button(button) => Box::new(button.clone()),
            UiElement::Toggle(toggle) => Box::new(toggle.clone()),
            UiElement::Lamp(lamp) => Box::new(lamp.clone()),
            UiElement::Slider(slider) => Box::new(slider.clone()),
        }
    }
}

/// One deserialized panel from the headless `--scene` `ui` JSON: a corner
/// `anchor` and `rows` of elements (outer array = top-to-bottom rows, inner =
/// left-to-right instruments). The owned, callback-free mirror of one
/// [`UIDrawablePanel`].
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPanel {
    pub anchor: PanelAnchor,
    pub rows: Vec<Vec<UiElement>>,
}

/// The set of deserialized `ui` panels, rendered through the exact same
/// [`crate::ui::control_panel`] path as the live UI so a mock layout is
/// faithful to real output. Every control is inert (no callback) - it renders
/// but does nothing.
pub struct PanelSet {
    pub panels: Vec<UiPanel>,
}

impl UIDrawable for PanelSet {
    /// Maps each owned [`UiPanel`] to a borrowed [`UIDrawablePanel`] of inert
    /// instruments - the same shape a live `get_drawables` returns, minus
    /// interactivity.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        self.panels
            .iter()
            .map(|panel| UIDrawablePanel {
                anchor: panel.anchor,
                rows: panel
                    .rows
                    .iter()
                    .map(|row| row.iter().map(UiElement::to_instrument).collect())
                    .collect(),
            })
            .collect()
    }
}
