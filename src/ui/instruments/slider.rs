use std::ops::RangeInclusive;

use serde::Deserialize;

use super::Instrument;

/// A value slider over `range` - the inert render data only. Drawing lives in
/// [`Slider::draw`], shared by this struct's read-only [`Instrument`] impl and
/// by [`InteractiveSlider`], which adds the edit callback.
///
/// `Deserialize` so the `render --scene` `ui` JSON can name it directly;
/// `Clone` so [`crate::ui::PanelSet`] can hand a copy out of its borrowing
/// `get_drawables`. `range` deserializes from a `[min, max]` JSON array (see
/// [`deserialize_range`]).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slider {
    pub position: [f32; 2],
    pub value: f32,
    #[serde(deserialize_with = "deserialize_range")]
    pub range: RangeInclusive<f32>,
}

impl Slider {
    /// Draws the slider into `ui`; returns the new value when the user edited
    /// it this frame, else `None`. Holds the widget look (width, no value
    /// label) so the inert and interactive paths render identically.
    fn draw(&self, ui: &mut egui::Ui) -> Option<f32> {
        ui.spacing_mut().slider_width = 280.0;
        let mut edited = self.value;
        ui.add(egui::Slider::new(&mut edited, self.range.clone()).show_value(false))
            .changed()
            .then_some(edited)
    }
}

impl Instrument for Slider {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        // Inert: still draggable, but the edit is discarded (e.g. a mock panel).
        let _ = self.draw(ui);
    }
}

/// A [`Slider`] wired to an edit callback. `on_change` receives the new value
/// on each edit; the producer owns any value mapping (e.g. the speed slider
/// edits an exponent and its callback exponentiates). The borrow `'a` is the
/// `&mut self` of the producing [`crate::ui::UIDrawable::get_drawables`], so
/// the closure can capture a disjoint mutable field of live state.
pub struct InteractiveSlider<'a> {
    pub slider: Slider,
    pub on_change: Box<dyn FnMut(f32) + 'a>,
}

impl Instrument for InteractiveSlider<'_> {
    fn position(&self) -> [f32; 2] {
        self.slider.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        if let Some(value) = self.slider.draw(ui) {
            (self.on_change)(value);
        }
    }
}

/// Reads a slider `range` from a `[min, max]` JSON array into a
/// `RangeInclusive`. The instrument keeps the richer range type; the wire form
/// stays the two-element array the `render --scene` `ui` JSON has always used.
fn deserialize_range<'de, D>(deserializer: D) -> Result<RangeInclusive<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let [min, max] = <[f32; 2]>::deserialize(deserializer)?;
    Ok(min..=max)
}
