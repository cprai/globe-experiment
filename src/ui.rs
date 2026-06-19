use crate::simulation::TelemetryState;
use crate::simulation::clock::Clock;

/// The control/readout panel, pinned to the top-left corner over the globe:
/// the simulation clock (play/pause + speed), the ephemeris-driven subsolar
/// point, and the tracked station's datetime and position.
///
/// The display values come from a [`TelemetryState`] snapshot the simulation
/// produced for this frame (alongside the `RenderState`), so the readout always
/// matches the rendered marker. The only thing the panel *mutates* is the
/// `Clock` (play/pause + speed), passed in by mutable reference.
///
/// The Sun's position is no longer user-controlled - it comes from the JPL
/// ephemeris for the clock's time (see `sky`), so the old latitude/longitude
/// sliders are gone.
pub fn control_panel(ctx: &egui::Context, telemetry: &TelemetryState, clock: &mut Clock) {
    egui::Area::new(egui::Id::new("control_panel"))
        .anchor(egui::Align2::LEFT_TOP, [10.0, 10.0])
        .show(ctx, |ui| {
            ui.set_width(260.0);
            ui.spacing_mut().slider_width = 260.0;

            // Sun: the ephemeris-derived subsolar point (read-only).
            ui.label(
                egui::RichText::new(format!(
                    "Sun (subsolar): lat {:.2} deg   lon {:.2} deg",
                    telemetry.subsolar_lat_deg, telemetry.subsolar_lon_deg
                ))
                .color(egui::Color32::WHITE),
            );

            ui.add_space(8.0);
            ui.separator();

            // Tracked station: the SGP4 prediction's datetime (driven by the
            // simulation clock) plus the resulting ground track and altitude.
            ui.label(
                egui::RichText::new(&telemetry.satellite_name)
                    .color(egui::Color32::from_rgb(255, 120, 100))
                    .strong(),
            );
            ui.label(
                egui::RichText::new(format!("Time (UTC): {}", telemetry.datetime_label))
                    .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(format!(
                    "Lat {:.2} deg   Lon {:.2} deg",
                    telemetry.latitude_deg, telemetry.longitude_deg
                ))
                .color(egui::Color32::WHITE),
            );
            ui.label(
                egui::RichText::new(format!("Altitude: {:.0} km", telemetry.altitude_km))
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
