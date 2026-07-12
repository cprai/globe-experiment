use egui::{CornerRadius, Margin, Stroke};
use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::{Instrument, leaf};
use crate::engine::ui::theme::{
    BEVEL_LIGHT, FONT_LABEL, FONT_VALUE, HAIRLINE, LABEL_DIM, RADIUS_UNIT, RADIUS_WINDOW,
    READOUT_CREAM, RECESS_FILL, SPACE_MD, SPACE_SM, SPACE_XS,
};

/// A labelled value: a dim caption above a large cream value in a recessed
/// digit window, with an optional unit stamped as an inverted block at the
/// window's end (e.g. `007.8|KM|`). The Python constructor's `unit` defaults
/// empty to mirror the serde default.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(from_py_object)]
pub struct Readout {
    #[pyo3(get, set)]
    pub label: String,
    #[pyo3(get, set)]
    pub value: String,
    #[serde(default)]
    #[pyo3(get, set)]
    pub unit: String,
}

#[pymethods]
impl Readout {
    #[new]
    #[pyo3(signature = (label, value, unit = String::new()))]
    fn py_new(label: String, value: String, unit: String) -> Self {
        Self { label, value, unit }
    }
}

impl<S> Instrument<S> for Readout {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Content-sized: the window hugs its fixed-width monospace value, so
        // the panel width comes from the widest readout row.
        leaf(tui, taffy::Style::default(), |ui| {
            readout_block(ui, &self.label, &self.value, &self.unit);
        });
    }
}

/// One digit-window readout (caption, recessed window, optional unit block);
/// shared with [`super::DualReadout`].
pub(super) fn readout_block(ui: &mut egui::Ui, label: &str, value: &str, unit: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = SPACE_XS;
        ui.label(
            egui::RichText::new(label.to_uppercase())
                .color(LABEL_DIM)
                .size(FONT_LABEL),
        );
        egui::Frame::new()
            .fill(RECESS_FILL)
            // The panel body and the recessed fill composite to nearly the
            // same on-screen value, so the outline - not the fill - is what
            // makes the window read as a cut-in field.
            .stroke(Stroke::new(HAIRLINE, BEVEL_LIGHT))
            .corner_radius(CornerRadius::same(RADIUS_WINDOW))
            // Slimmer right margin when a unit block caps the window, so it
            // sits flush at the end like a stamped suffix.
            .inner_margin(Margin {
                left: (SPACE_MD + HAIRLINE) as i8,
                right: if unit.is_empty() {
                    (SPACE_MD + HAIRLINE) as i8
                } else {
                    SPACE_SM as i8
                },
                top: SPACE_SM as i8,
                bottom: SPACE_SM as i8,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_MD;
                    ui.label(
                        egui::RichText::new(value.to_uppercase())
                            .color(READOUT_CREAM)
                            .size(FONT_VALUE),
                    );
                    if !unit.is_empty() {
                        egui::Frame::new()
                            .fill(READOUT_CREAM)
                            .corner_radius(CornerRadius::same(RADIUS_UNIT))
                            .inner_margin(Margin::symmetric(SPACE_SM as i8, HAIRLINE as i8))
                            .show(ui, |ui| {
                                ui.label(
                                    egui::RichText::new(unit.to_uppercase())
                                        .color(RECESS_FILL)
                                        .strong()
                                        .size(FONT_LABEL),
                                );
                            });
                    }
                });
            });
    });
}
