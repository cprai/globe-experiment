use super::Instrument;

/// A value slider over `range`. `on_change` receives the new value on edit
/// (when `Some`); a `None` callback renders an inert slider (e.g. for a mock
/// panel). The producer owns any value mapping (e.g. the speed slider edits an
/// exponent and its callback exponentiates).
pub struct Slider<'a> {
    pub position: [f32; 2],
    pub value: f32,
    pub range: std::ops::RangeInclusive<f32>,
    pub on_change: Option<Box<dyn FnMut(f32) + 'a>>,
}

impl Instrument for Slider<'_> {
    fn position(&self) -> [f32; 2] {
        self.position
    }

    fn render(&mut self, ui: &mut egui::Ui, _child_rect: egui::Rect, _panel_size: egui::Vec2) {
        ui.spacing_mut().slider_width = 280.0;
        let mut edited = self.value;
        if ui
            .add(egui::Slider::new(&mut edited, self.range.clone()).show_value(false))
            .changed()
            && let Some(callback) = self.on_change.as_mut()
        {
            callback(edited);
        }
    }
}
