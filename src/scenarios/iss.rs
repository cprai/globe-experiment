//! ISS-only scenario: track the International Space Station from its
//! ~2024-001.5 TLE epoch. Same as `iss_and_hubble` but with Hubble omitted, so
//! a single marker renders (CLI: `globe-experiment scenario iss`).

use glam::{Mat3, Mat4, Vec3};

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{
    self, RenderState, SatelliteMarker, SatelliteTelemetry, Simulation, SimulationState,
    marker_occluded,
};
use crate::ui::{PanelAnchor, UIDrawable, UIDrawableElement, UIDrawablePanel};

// This scenario's tracked-object TLE, inlined as a source literal. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), this small element set lives directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from this via `Satellite::from_tle`. (Deliberately duplicated from
// `iss_and_hubble.rs` - each scenario owns its own TLE data.)

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// ISS-only simulation: the shared core (clock + celestial sphere) via
/// composition, plus this scenario's single tracked satellite.
pub struct IssSimulation {
    simulation: SimulationState,
    satellites: Vec<Satellite>,
    /// Per-satellite readout from this frame's propagation, stashed by
    /// `frame_state` for the immediately-following `get_drawables` (the panel),
    /// so the readout matches the rendered markers. Empty until the first
    /// frame.
    last_telemetry: Vec<SatelliteTelemetry>,
}

impl IssSimulation {
    fn new() -> Self {
        let satellites = vec![Satellite::from_tle(ISS_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            simulation: SimulationState::new(epoch),
            satellites,
            last_telemetry: Vec::new(),
        }
    }
}

impl Simulation for IssSimulation {
    fn advance(&mut self) -> bool {
        self.simulation.advance()
    }

    fn celestial_to_world(&self) -> Mat3 {
        self.simulation.celestial_to_world()
    }

    fn frame_state(&mut self, eye: Vec3, view_proj: Mat4) -> RenderState {
        let now = self.simulation.clock.now();

        let mut markers = Vec::with_capacity(self.satellites.len());
        let mut sat_telemetry = Vec::with_capacity(self.satellites.len());
        for sat in &mut self.satellites {
            let state = sat.state_at(&now);
            markers.push(SatelliteMarker {
                position_km: state.position_km,
                visible: !marker_occluded(eye, state.position_km),
            });
            sat_telemetry.push(SatelliteTelemetry {
                name: sat.name.clone(),
                latitude_deg: state.latitude_deg,
                longitude_deg: state.longitude_deg,
                altitude_km: state.altitude_km,
            });
        }

        // Stash for this frame's `get_drawables` (the panel), so the readout
        // comes from the same propagation as the markers.
        self.last_telemetry = sat_telemetry;

        RenderState {
            view_proj,
            camera_pos: eye,
            sun_dir: self.simulation.celestial_sphere.sun_dir,
            star_rot_inv: self.simulation.celestial_sphere.star_rot_inv,
            markers,
        }
    }
}

impl UIDrawable for IssSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel first (its callbacks borrow `self.simulation`),
        // then this scenario's own panel built from the disjoint
        // `self.last_telemetry` field (so it coexists with that borrow). The
        // readout loop is deliberately kept per-scenario (like the propagation
        // loop) - scenarios may diverge in how they present objects.
        let mut panels = self.simulation.get_drawables();
        let mut elements = Vec::with_capacity(self.last_telemetry.len() * 3);
        for (index, sat) in self.last_telemetry.iter().enumerate() {
            let top = index as f32 * 74.0;
            elements.push(UIDrawableElement::Header {
                position: [0.0, top],
                title: sat.name.clone(),
            });
            elements.push(UIDrawableElement::DualReadout {
                position: [0.0, top + 24.0],
                left_label: "Lat".to_string(),
                left_value: format!("{:.2} deg", sat.latitude_deg),
                right_label: "Lon".to_string(),
                right_value: format!("{:.2} deg", sat.longitude_deg),
            });
            elements.push(UIDrawableElement::Readout {
                position: [0.0, top + 48.0],
                label: "Alt".to_string(),
                value: format!("{:.0} km", sat.altitude_km),
            });
        }
        panels.push(UIDrawablePanel {
            anchor: PanelAnchor::TopRight,
            offset: [10.0, 10.0],
            size: [300.0, self.last_telemetry.len() as f32 * 74.0 + 16.0],
            elements,
        });
        panels
    }
}

/// Builds the ISS simulation and hands off to the winit event loop. Blocks
/// until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: IssSimulation::new below builds the CelestialSphere
    // (which reads the ephemeris) and the satellite parses a TLE. Doing it
    // here keeps satkit fully offline and data-dir-free.
    simulation::init();

    application::run(ApplicationState::new(IssSimulation::new()));
}
