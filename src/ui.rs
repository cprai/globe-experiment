use crate::globe::clock::Clock;
use crate::globe::satellite::Satellite;
use crate::globe::sun::Sun;

/// The control/readout panel, pinned to the top-left corner over the globe:
/// the sun sliders, the simulation clock (play/pause + speed), and the
/// tracked station's datetime and position.
///
/// Solar declination spans +/-23.44 deg over the year; the subsolar longitude
/// sweeps the full globe over a day.
pub fn sun_panel(ctx: &egui::Context, sun: &mut Sun, satellite: &Satellite, clock: &mut Clock) {
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

            ui.add_space(8.0);
            ui.separator();

            // Tracked station: the SGP4 prediction's datetime (driven by the
            // simulation clock) plus the resulting ground track and altitude.
            ui.label(
                egui::RichText::new(&satellite.name)
                    .color(egui::Color32::from_rgb(255, 120, 100))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!("Time (UTC): {}", clock.datetime_label()))
                    .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Lat {:.2} deg   Lon {:.2} deg",
                    satellite.latitude_deg, satellite.longitude_deg
                ))
                .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(format!("Altitude: {:.0} km", satellite.altitude_km))
                    .color(egui::Color32::WHITE),
            );

            // Clock controls: play/pause and a real-time..100x speed slider.
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if clock.paused { "Play" } else { "Pause" };
                if ui.button(label).clicked() {
                    clock.paused = !clock.paused;
                }
                ui.label(
                    egui::RichText::new(format!("Speed: {:.1}x", clock.multiplier))
                        .color(egui::Color32::WHITE),
                );
            });
            // Exponential (base e) speed: the slider edits the exponent, so
            // multiplier = e^exp. Linear travel on the slider therefore scales
            // time geometrically - real time (e^0 = 1x) at the left up to 100x
            // (e^ln100) at the right, with 10x at the midpoint. Write back only
            // on change, so the multiplier->exp->multiplier round trip never
            // drifts when the slider is idle.
            let mut speed_exp = clock.multiplier.ln();
            let exp_range = Clock::MIN_MULTIPLIER.ln()..=Clock::MAX_MULTIPLIER.ln();
            if ui
                .add(egui::Slider::new(&mut speed_exp, exp_range).show_value(false))
                .changed()
            {
                clock.multiplier = speed_exp.exp();
            }
        });
}
