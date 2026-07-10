use egui::Stroke;
use egui_taffy::{Tui, TuiBuilderLogic, taffy};
use pyo3::prelude::*;

use super::{Callback, Instrument};
use crate::engine::ui::theme::{ACCENT_GREEN, HAIRLINE, KEY_LIT, KEY_LIT_TEXT};

/// A latching key that lights green while `active` - the inert render data
/// only. Drawing lives in [`Toggle::draw`], shared by this struct's read-only
/// [`Instrument`] impl and by [`InteractiveToggle`], which adds the toggle
/// callback.
///
/// `Deserialize` so the headless `--scene` `ui` JSON can name it directly (the
/// `active` flag still drives its lit look); `Clone` so
/// [`crate::engine::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables` (and the Python bridge out of its pyclass cell); `pyclass`
/// for the dual Rust/Python UI API.
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
    /// was clicked this frame. Holds the lit-look override so the inert and
    /// interactive paths render identically. When lit, every pointer state
    /// (rest/hover/press) is forced to the lit look so the key reads as an
    /// engaged lamp, not a momentary button; the style override rides on this
    /// node only.
    fn draw(&self, tui: &mut Tui) -> bool {
        let text = egui::RichText::new(self.label.to_uppercase());
        // Keys grow to share their row: one key per row fills the panel (the
        // selector column look); keys sharing a row split it evenly.
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

/// The shared key node style: grow to share the row with siblings (a lone key
/// spans the panel; paired keys split it).
pub(super) fn key_style() -> taffy::Style {
    taffy::Style {
        flex_grow: 1.0,
        ..Default::default()
    }
}

impl<S> Instrument<S> for Toggle {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Inert: still clickable, but the click does nothing (e.g. a mock panel).
        let _ = self.draw(tui);
    }
}

/// A [`Toggle`] wired to a toggle callback. `on_toggle` fires on click,
/// receiving the live scene (the producer's `&mut self`, threaded in by
/// `control_panel` at fire time - no capture, so every panel callback
/// coexists); the producer owns flipping the `active` state it reflects.
/// The callback must be idempotent (set a value snapshotted at panel-build
/// time, e.g. `move |scene| scene.set_clock_paused(running)` - never flip
/// live state it re-reads): egui's discard pass can fire it twice per frame.
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
