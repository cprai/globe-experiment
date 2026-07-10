use egui::Stroke;
use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::{Instrument, leaf};
use crate::engine::ui::theme::{
    ACCENT_GREEN, ACCENT_RED, BEVEL_DARK, HAIRLINE, HEADER_AMBER, LABEL_DIM, RECESS_FILL, SPACE_LG,
    SPACE_MD, SPACE_SM,
};

/// The *semantic* condition a producer selects for a [`Lamp`], mapped to a lamp
/// color in [`Lamp::render`] (the producer never names a color). Serde
/// snake_case so the mock JSON can say `"status": "ok"`.
///
/// No live panel constructs a lamp today - it is part of the reusable
/// instrument library, constructed only via the headless binary's `ui` spec
/// or a scene script (`pyclass`: the variants surface in Python as
/// `LampStatus.Ok` etc., `eq` so scripts can compare them) - so `dead_code`
/// is allowed in the main binary's tree until a producer uses one.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[pyclass(module = "globe", eq, from_py_object)]
pub enum LampStatus {
    /// Nominal - green.
    Ok,
    /// Caution - amber.
    Caution,
    /// Fault - red.
    Fault,
    /// Unlit/inactive - dim.
    Off,
}

/// A status indicator lamp: a colored disc in a recessed socket keyed to
/// `status`, plus a dim caption.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::engine::ui::PanelSet`] can hand a copy out of its
/// borrowing `get_drawables` (and the Python bridge out of its pyclass cell);
/// `pyclass` for the dual Rust/Python UI API. `dead_code` allowed like
/// [`LampStatus`] (no live producer).
#[allow(dead_code)]
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(module = "globe", from_py_object)]
pub struct Lamp {
    #[pyo3(get, set)]
    pub label: String,
    #[pyo3(get, set)]
    pub status: LampStatus,
}

#[pymethods]
impl Lamp {
    #[new]
    fn py_new(label: String, status: LampStatus) -> Self {
        Self { label, status }
    }
}

impl<S> Instrument<S> for Lamp {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // The `status` picks the lamp color here - the producer only names the
        // condition.
        let color = match self.status {
            LampStatus::Ok => ACCENT_GREEN,
            LampStatus::Caution => HEADER_AMBER,
            LampStatus::Fault => ACCENT_RED,
            LampStatus::Off => LABEL_DIM,
        };
        // Lamp geometry off the spacing scale: LG-radius socket box, MD socket,
        // SM lit disc, with the halo splitting socket and disc.
        let socket_radius = SPACE_MD;
        let halo_radius = SPACE_MD - HAIRLINE;
        let disc_radius = SPACE_SM;
        leaf(tui, taffy::Style::default(), |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(SPACE_LG * 2.0, SPACE_LG * 2.0),
                    egui::Sense::hover(),
                );
                let center = rect.center();
                let painter = ui.painter();
                painter.circle_filled(center, socket_radius, RECESS_FILL);
                // A soft halo, then the lit disc.
                painter.circle_filled(center, halo_radius, color.gamma_multiply(0.35));
                painter.circle_filled(center, disc_radius, color);
                painter.circle_stroke(center, socket_radius, Stroke::new(HAIRLINE, BEVEL_DARK));
                ui.label(egui::RichText::new(self.label.to_uppercase()).color(LABEL_DIM));
            });
        });
    }
}
