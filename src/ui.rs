use crate::globe::sun::Sun;

/// The sun control panel, pinned to the top-left corner over the globe.
///
/// Solar declination spans +/-23.44 deg over the year; the subsolar longitude
/// sweeps the full globe over a day.
pub fn sun_panel(ctx: &egui::Context, sun: &mut Sun) {
    egui::Area::new(egui::Id::new("sun_control"))
        .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
        .show(ctx, |ui| {
            ui.set_width(260.0);
            ui.spacing_mut().slider_width = 260.0;

            ui.label(
                egui::RichText::new(format!("Sun latitude: {:.1} deg", sun.latitude))
                    .color(egui::Color32::WHITE),
            );
            ui.add(
                egui::Slider::new(&mut sun.latitude, -23.44..=23.44)
                    .step_by(0.1)
                    .show_value(false),
            );

            ui.label(
                egui::RichText::new(format!("Sun longitude: {:.1} deg", sun.longitude))
                    .color(egui::Color32::WHITE),
            );
            ui.add(
                egui::Slider::new(&mut sun.longitude, -180.0..=180.0)
                    .step_by(0.5)
                    .show_value(false),
            );
        });
}
