use egui::Stroke;
use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;

use super::{Instrument, leaf};
use crate::engine::ui::theme::{
    BEVEL_LIGHT, FONT_TITLE, HAIRLINE, HEADER_AMBER, SPACE_SM, SPACE_XS,
};

/// A section header: a bold amber title with a rule ruled across the panel
/// width beneath it - the labelled divider that tops each cluster on the Apollo
/// panels.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::engine::ui::PanelSet`] can hand a copy out of its
/// borrowing `get_drawables` (and so the Python bridge can clone one out of
/// its pyclass cell); `pyclass` for the dual Rust/Python UI API (a scene
/// script builds the same bare struct - see `engine::ui::py`).
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(module = "globe", from_py_object)]
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
            // includes it; the rule itself spans the node's full (row-grown)
            // width, not just the title's.
            ui.add_space(SPACE_SM);
            ui.painter().hline(
                ui.max_rect().x_range(),
                title.rect.bottom() + SPACE_XS,
                Stroke::new(HAIRLINE, BEVEL_LIGHT),
            );
        });
    }
}
