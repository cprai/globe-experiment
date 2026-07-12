use std::ops::RangeInclusive;

use egui_taffy::{Tui, taffy};
use pyo3::prelude::*;
use serde::Deserialize;

use super::{Instrument, ValueCallback, leaf};

/// A value slider over `range` - the inert render data only; [`Slider::draw`]
/// is shared with [`InteractiveSlider`]. `range` deserializes from a
/// `[min, max]` JSON array; `RangeInclusive` has no pyo3 conversion, so its
/// Python face is a `(min, max)` tuple (explicit getter/setter below).
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[pyclass(from_py_object)]
pub struct Slider {
    #[pyo3(get, set)]
    pub value: f32,
    #[serde(deserialize_with = "deserialize_range")]
    pub range: RangeInclusive<f32>,
}

#[pymethods]
impl Slider {
    #[new]
    fn py_new(value: f32, range: (f32, f32)) -> Self {
        Self {
            value,
            range: range.0..=range.1,
        }
    }

    #[getter(range)]
    fn py_range(&self) -> (f32, f32) {
        (*self.range.start(), *self.range.end())
    }

    #[setter(range)]
    fn py_set_range(&mut self, range: (f32, f32)) {
        self.range = range.0..=range.1;
    }
}

impl Slider {
    /// Adds the slider as a full-width flex node; returns the new value when
    /// the user edited it this frame, else `None`.
    fn draw(&self, tui: &mut Tui) -> Option<f32> {
        // Percent width (not grow): a percentage contributes nothing to the
        // panel's content-driven min width, so the track follows whatever
        // width the readout rows set rather than driving it.
        let style = taffy::Style {
            size: taffy::Size {
                width: taffy::prelude::percent(1.0_f32),
                height: taffy::prelude::auto(),
            },
            ..Default::default()
        };
        leaf(tui, style, |ui| {
            // The track spans the node (= the panel's content width).
            ui.spacing_mut().slider_width = ui.available_width();
            let mut edited = self.value;
            ui.add(egui::Slider::new(&mut edited, self.range.clone()).show_value(false))
                .changed()
                .then_some(edited)
        })
    }
}

impl<S> Instrument<S> for Slider {
    fn render(&mut self, tui: &mut Tui, _scene: &mut S) {
        // Inert: still draggable, but the edit is discarded (mock panels).
        let _ = self.draw(tui);
    }
}

/// A [`Slider`] wired to an `on_change(scene, value)` edit callback; the
/// producer owns any value mapping (e.g. the speed slider edits an exponent).
/// Writing the received value is naturally idempotent under egui's
/// discard-pass double fire.
pub struct InteractiveSlider<S> {
    pub slider: Slider,
    pub on_change: ValueCallback<S>,
}

impl<S> Instrument<S> for InteractiveSlider<S> {
    fn render(&mut self, tui: &mut Tui, scene: &mut S) {
        if let Some(value) = self.slider.draw(tui) {
            (self.on_change)(scene, value);
        }
    }
}

/// Reads a slider `range` from a `[min, max]` JSON array.
fn deserialize_range<'de, D>(deserializer: D) -> Result<RangeInclusive<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let [min, max] = <[f32; 2]>::deserialize(deserializer)?;
    Ok(min..=max)
}
