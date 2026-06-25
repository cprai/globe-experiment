//! The `render --scene` `ui` overlay: a serde-deserialized, callback-free
//! mirror of the live panels, run through the exact same
//! [`crate::ui::control_panel`] path so a mock layout is faithful to real
//! output.

use super::instruments::{
    Button, DualReadout, Header, Instrument, Lamp, LampStatus, Readout, Slider, Toggle,
};
use super::{PanelAnchor, UIDrawable, UIDrawablePanel};

/// Owned, callback-free mirror of one instrument, deserialized from the
/// `render --scene` `ui` JSON. The live control instruments carry `Box<dyn
/// FnMut>` callbacks (not serializable); a mock panel is inert, so these carry
/// only the rendered data (a `Toggle`'s `active` still drives its lit look).
/// Tagged by instrument name in snake_case, e.g.
/// `{"readout": {"position": [0, 0], "label": "ALT", "value": "417 km"}}`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum UiElementSpec {
    /// A section header.
    Header { position: [f32; 2], title: String },
    /// A labelled value readout.
    Readout {
        position: [f32; 2],
        label: String,
        value: String,
    },
    /// Two labelled values on one row.
    DualReadout {
        position: [f32; 2],
        left_label: String,
        left_value: String,
        right_label: String,
        right_value: String,
    },
    /// An inert momentary key (renders, does nothing).
    Button { position: [f32; 2], label: String },
    /// An inert latching key; `active` drives the lit look.
    Toggle {
        position: [f32; 2],
        label: String,
        active: bool,
    },
    /// A status indicator lamp.
    Lamp {
        position: [f32; 2],
        label: String,
        status: LampStatus,
    },
    /// An inert slider. `range` is `[min, max]`.
    Slider {
        position: [f32; 2],
        value: f32,
        range: [f32; 2],
    },
}

impl UiElementSpec {
    /// Builds the matching inert ([`None`]-callback) instrument trait object.
    fn to_instrument(&self) -> Box<dyn Instrument> {
        match self {
            UiElementSpec::Header { position, title } => Box::new(Header {
                position: *position,
                title: title.clone(),
            }),
            UiElementSpec::Readout {
                position,
                label,
                value,
            } => Box::new(Readout {
                position: *position,
                label: label.clone(),
                value: value.clone(),
            }),
            UiElementSpec::DualReadout {
                position,
                left_label,
                left_value,
                right_label,
                right_value,
            } => Box::new(DualReadout {
                position: *position,
                left_label: left_label.clone(),
                left_value: left_value.clone(),
                right_label: right_label.clone(),
                right_value: right_value.clone(),
            }),
            UiElementSpec::Button { position, label } => Box::new(Button {
                position: *position,
                label: label.clone(),
                on_press: None,
            }),
            UiElementSpec::Toggle {
                position,
                label,
                active,
            } => Box::new(Toggle {
                position: *position,
                label: label.clone(),
                active: *active,
                on_toggle: None,
            }),
            UiElementSpec::Lamp {
                position,
                label,
                status,
            } => Box::new(Lamp {
                position: *position,
                label: label.clone(),
                status: *status,
            }),
            UiElementSpec::Slider {
                position,
                value,
                range,
            } => Box::new(Slider {
                position: *position,
                value: *value,
                range: range[0]..=range[1],
                on_change: None,
            }),
        }
    }
}

/// Owned, callback-free mirror of one [`UIDrawablePanel`], deserialized from
/// the `render --scene` `ui` JSON: a corner `anchor`, an inset `offset`, a
/// fixed box `size`, and panel-relative `elements`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiPanelSpec {
    pub anchor: PanelAnchor,
    pub offset: [f32; 2],
    pub size: [f32; 2],
    pub elements: Vec<UiElementSpec>,
}

/// A set of mock panels (from the `render --scene` `ui` section) that renders
/// through the exact same [`crate::ui::control_panel`] path as the live UI, so
/// a mock layout is faithful to real output. Every control is inert: its
/// callback is `None`.
pub struct MockUi {
    pub panels: Vec<UiPanelSpec>,
}

impl UIDrawable for MockUi {
    /// Maps each owned [`UiPanelSpec`] to a borrowed [`UIDrawablePanel`] with
    /// every control inert - the same shape a live `get_drawables` returns,
    /// minus interactivity.
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        self.panels
            .iter()
            .map(|panel| UIDrawablePanel {
                anchor: panel.anchor,
                offset: panel.offset,
                size: panel.size,
                elements: panel
                    .elements
                    .iter()
                    .map(UiElementSpec::to_instrument)
                    .collect(),
            })
            .collect()
    }
}
