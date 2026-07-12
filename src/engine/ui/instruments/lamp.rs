use egui::Stroke;
use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::{Instrument, leaf};
use crate::engine::ui::theme::{
    ACCENT_GREEN, ACCENT_RED, BEVEL_DARK, HAIRLINE, HEADER_AMBER, LABEL_DIM, RECESS_FILL, SPACE_LG,
    SPACE_MD, SPACE_SM,
};

/// The *semantic* condition a producer selects for a [`Lamp`]; the render
/// maps it to a color (a producer never names one).
///
/// No live panel constructs a lamp yet (headless spec / scripts only), so
/// `dead_code` is allowed until a producer uses one.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[pyclass(eq, from_py_object)]
pub enum LampStatus {
    Ok,
    Caution,
    Fault,
    Off,
}

/// A status indicator lamp: a colored disc in a recessed socket keyed to
/// `status`, plus a dim caption. `dead_code` allowed like [`LampStatus`].
#[allow(dead_code)]
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(from_py_object)]
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
        let color = match self.status {
            LampStatus::Ok => ACCENT_GREEN,
            LampStatus::Caution => HEADER_AMBER,
            LampStatus::Fault => ACCENT_RED,
            LampStatus::Off => LABEL_DIM,
        };
        // Geometry off the spacing scale: LG-radius socket box, MD socket,
        // SM lit disc, the halo splitting socket and disc.
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
                painter.circle_filled(center, halo_radius, color.gamma_multiply(0.35));
                painter.circle_filled(center, disc_radius, color);
                painter.circle_stroke(center, socket_radius, Stroke::new(HAIRLINE, BEVEL_DARK));
                ui.label(egui::RichText::new(self.label.to_uppercase()).color(LABEL_DIM));
            });
        });
    }
}
