use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::readout::readout_block;
use super::{Instrument, leaf};
use crate::engine::ui::theme::SPACE_XXL;

/// Two labelled values side by side on one row, for compact paired readouts
/// (e.g. LAT / LON). Reuses [`super::Readout`]'s digit window (label above,
/// value + optional unit block below) for each half.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly (the
/// units default empty, so pre-unit JSON still parses); `Clone` so
/// [`crate::engine::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables` (and the Python bridge out of its pyclass cell); `pyclass`
/// for the dual Rust/Python UI API - the Python constructor interleaves the
/// units (`left_label, left_value, left_unit, right_...`) to match the field
/// order, with the units defaulting empty like the serde defaults.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(module = "globe", from_py_object)]
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

impl Instrument for DualReadout {
    fn render(&mut self, tui: &mut Tui) {
        leaf(tui, taffy::Style::default(), |ui| {
            ui.horizontal_top(|ui| {
                readout_block(ui, &self.left_label, &self.left_value, &self.left_unit);
                // A wider-than-row gap between the pair, so the two windows
                // read as one paired instrument rather than two columns.
                ui.add_space(SPACE_XXL);
                readout_block(ui, &self.right_label, &self.right_value, &self.right_unit);
            });
        });
    }
}
