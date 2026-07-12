use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::readout::readout_block;
use super::{Instrument, leaf};
use crate::engine::ui::theme::SPACE_XXL;

/// Two labelled values side by side (e.g. LAT / LON), reusing
/// [`super::Readout`]'s digit window per half. The Python constructor
/// interleaves the units to match the field order, with the units defaulting
/// empty like the serde defaults.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(from_py_object)]
pub struct DualReadout {
    #[pyo3(get, set)]
    pub left_label: String,
    #[pyo3(get, set)]
    pub left_value: String,
    #[serde(default)]
    #[pyo3(get, set)]
    pub left_unit: String,
    #[pyo3(get, set)]
    pub right_label: String,
    #[pyo3(get, set)]
    pub right_value: String,
    #[serde(default)]
    #[pyo3(get, set)]
    pub right_unit: String,
}

#[pymethods]
impl DualReadout {
    #[new]
    #[pyo3(signature = (left_label, left_value, left_unit, right_label, right_value, right_unit = String::new()))]
    fn py_new(
        left_label: String,
        left_value: String,
        left_unit: String,
        right_label: String,
        right_value: String,
        right_unit: String,
    ) -> Self {
        Self {
            left_label,
            left_value,
            left_unit,
            right_label,
            right_value,
            right_unit,
        }
    }
}

impl<S> Instrument<S> for DualReadout {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        leaf(tui, taffy::Style::default(), |ui| {
            ui.horizontal_top(|ui| {
                readout_block(ui, &self.left_label, &self.left_value, &self.left_unit);
                // Wider-than-row gap so the pair reads as one instrument,
                // not two columns.
                ui.add_space(SPACE_XXL);
                readout_block(ui, &self.right_label, &self.right_value, &self.right_unit);
            });
        });
    }
}
