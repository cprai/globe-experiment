//! The headless `--scene` `ui` overlay: panels deserialized from the scene
//! JSON and rendered through the same `control_panel` path as the live app,
//! so a mock layout is faithful to real output. The bare instrument structs
//! derive `Deserialize` themselves, so this is just a tagged enum over them -
//! no field-duplicating mirror type, and every control is inert.

use super::instruments::{Button, DualReadout, Header, Instrument, Lamp, Readout, Slider, Toggle};
use super::{PanelAnchor, UIDrawable, UIDrawablePanel};

/// One deserialized instrument, tagged by snake_case instrument name, e.g.
/// `{"readout": {"label": "ALT", "value": "417", "unit": "km"}}`.
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
    /// Boxes a clone of the wrapped instrument (cloned, not moved, because
    /// [`PanelSet::get_drawables`] runs every frame off a shared borrow).
    fn to_instrument<S>(&self) -> Box<dyn Instrument<S>> {
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

/// One deserialized panel: a corner `anchor` and `rows` of elements (outer =
/// top-to-bottom rows, inner = left-to-right instruments; no pixel
/// coordinates).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPanel {
    pub anchor: PanelAnchor,
    pub rows: Vec<Vec<UiElement>>,
}

/// The set of deserialized `ui` panels; every control is inert (no callback).
pub struct PanelSet {
    pub panels: Vec<UiPanel>,
}

impl UIDrawable for PanelSet {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<Self>> {
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
