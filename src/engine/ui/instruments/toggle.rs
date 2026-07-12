use egui::Stroke;
use egui_taffy::{Tui, TuiBuilderLogic, taffy};
use pyo3::prelude::*;

use super::{Callback, Instrument};
use crate::engine::ui::theme::{ACCENT_GREEN, HAIRLINE, KEY_LIT, KEY_LIT_TEXT};

/// A latching key that lights green while `active` - the inert render data
/// only; [`Toggle::draw`] is shared with [`InteractiveToggle`].
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(module = "globe", from_py_object)]
pub struct Toggle {
    #[pyo3(get, set)]
    pub label: String,
    #[pyo3(get, set)]
    pub active: bool,
}

#[pymethods]
impl Toggle {
    #[new]
    fn py_new(label: String, active: bool) -> Self {
        Self { label, active }
    }
}

impl Toggle {
    /// Adds the latching key as its own grown flex node; returns whether it
    /// was clicked. When lit, every pointer state (rest/hover/press) is
    /// forced to the lit look so the key reads as an engaged lamp, not a
    /// momentary button; the style override rides on this node only.
    fn draw(&self, tui: &mut Tui) -> bool {
        let text = egui::RichText::new(self.label.to_uppercase());
        let node = tui.style(key_style());
        if !self.active {
            return node.ui_add(egui::Button::new(text)).clicked();
        }
        node.mut_egui_style(|style| {
            let widgets = &mut style.visuals.widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
            ] {
                state.bg_fill = KEY_LIT;
                state.weak_bg_fill = KEY_LIT;
                state.bg_stroke = Stroke::new(HAIRLINE, ACCENT_GREEN);
                state.fg_stroke = Stroke::new(HAIRLINE, KEY_LIT_TEXT);
            }
        })
        .ui_add(egui::Button::new(text.color(KEY_LIT_TEXT).strong()))
        .clicked()
    }
}

/// The shared key node style: grow to share the row (a lone key spans the
/// panel; paired keys split it).
pub(super) fn key_style() -> taffy::Style {
    taffy::Style {
        flex_grow: 1.0,
        ..Default::default()
    }
}

impl<S> Instrument<S> for Toggle {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Inert: still clickable, but the click does nothing (mock panels).
        let _ = self.draw(tui);
    }
}

/// A [`Toggle`] wired to an `on_toggle` click callback; the producer owns
/// flipping the `active` state it reflects. The callback must be idempotent -
/// write a build-time snapshot (e.g. `move |scene|
/// scene.set_clock_paused(running)`), never flip live state it re-reads: the
/// discard pass can fire it twice per frame.
pub struct InteractiveToggle<S> {
    pub toggle: Toggle,
    pub on_toggle: Callback<S>,
}

impl<S> Instrument<S> for InteractiveToggle<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S) {
        if self.toggle.draw(tui) {
            (self.on_toggle)(scene);
        }
    }
}
