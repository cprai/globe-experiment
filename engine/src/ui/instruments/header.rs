use egui::Stroke;
use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::{Instrument, leaf};
use crate::ui::theme::{BEVEL_LIGHT, FONT_TITLE, HAIRLINE, HEADER_AMBER, SPACE_SM, SPACE_XS};

/// A section header: an amber title with a rule spanning the panel width.
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(from_py_object)]
pub struct Header {
    #[pyo3(get, set)]
    pub title: String,
}

#[pymethods]
impl Header {
    #[new]
    fn py_new(title: String) -> Self {
        Self { title }
    }
}

impl<S> Instrument<S> for Header {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Grow across the row so the rule spans the full panel width.
        let style = taffy::Style {
            flex_grow: 1.0,
            ..Default::default()
        };
        leaf(tui, style, |ui| {
            let title = ui.label(
                egui::RichText::new(self.title.to_uppercase())
                    .color(HEADER_AMBER)
                    .strong()
                    .size(FONT_TITLE),
            );
            // Reserve the rule's strip below the title so the node's height
            // includes it; the rule spans the node's full (row-grown) width.
            ui.add_space(SPACE_SM);
            ui.painter().hline(
                ui.max_rect().x_range(),
                title.rect.bottom() + SPACE_XS,
                Stroke::new(HAIRLINE, BEVEL_LIGHT),
            );
        });
    }
}
