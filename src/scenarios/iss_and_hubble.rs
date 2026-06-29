//! ISS + Hubble scenario: track the International Space Station and the Hubble
//! Space Telescope from their shared ~2024-001.5 TLE epoch. This is the
//! original default scene, now expressed as a named scenario (CLI:
//! `globe-experiment scenario iss_and_hubble`).

use glam::{Mat3, Mat4, Vec3};

use crate::application::{self, ApplicationState};
use crate::simulation::satellite::Satellite;
use crate::simulation::{
    self, RenderState, SatelliteMarker, SatelliteTelemetry, Simulation, SimulationState,
    marker_occluded,
};
use crate::ui::{
    DualReadout, Header, Instrument, PanelAnchor, Readout, UIDrawable, UIDrawablePanel,
};

// This scenario's tracked-object TLEs, inlined as source literals. Unlike the
// textures/ephemeris/EOP (build-downloaded straight into `OUT_DIR` and baked
// into the binary), these small element sets live directly in source so a fresh
// checkout needs no data file. The lines are column-sensitive TLE format (each
// element line is exactly 69 chars) - keep the exact spacing. `concat!` keeps
// source indentation out of the string. satkit parses by column and does not
// verify the trailing checksum digit. `new` below assembles the tracked array
// from these via `Satellite::from_tle`.

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// The Hubble Space Telescope (HST), epoch ~2024-001.5. NOTE: the orbit shape
/// is realistic (inclination 28.47 deg, ~540 km / ~15.1 rev/day), but this set
/// was assembled from memory - the RAAN/anomaly/epoch-fraction phase is
/// approximate. Replace with a freshly fetched TLE for true positional
/// accuracy. Included as a second tracked object so multiple markers render.
const HST_TLE: &str = concat!(
    "HST\n",
    "1 20580U 90037B   24001.49473380  .00002000  00000-0  10000-3 0  9990\n",
    "2 20580  28.4690  85.5400 0002600 310.0000  50.0000 15.09600000123456\n",
);

/// ISS + Hubble simulation: the shared core (clock + celestial sphere) via
/// composition, plus this scenario's two tracked satellites.
pub struct IssAndHubbleSimulation {
    simulation: SimulationState,
    satellites: Vec<Satellite>,
    /// Per-satellite readout from this frame's propagation, stashed by
    /// `frame_state` for the immediately-following `get_drawables` (the panel),
    /// so the readout matches the rendered markers. Empty until the first
    /// frame.
    last_telemetry: Vec<SatelliteTelemetry>,
}

impl IssAndHubbleSimulation {
    fn new() -> Self {
        // The clock starts at the first satellite's TLE epoch, so order
        // matters: the primary object (ISS) goes first.
        let satellites = vec![Satellite::from_tle(ISS_TLE), Satellite::from_tle(HST_TLE)];
        let epoch = satellites.first().expect("TLE present").epoch();
        Self {
            simulation: SimulationState::new(epoch),
            satellites,
            last_telemetry: Vec::new(),
        }
    }
}

impl Simulation for IssAndHubbleSimulation {
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

        let celestial = &self.simulation.celestial_sphere;
        RenderState {
            view_proj,
            camera_pos: eye,
            // Terra target: the origin stays at Terra (planet-free scene).
            render_origin: Vec3::ZERO,
            sol_pos_world: celestial.sol_pos_world,
            star_rot_inv: celestial.star_tex_rot_inv,
            // The Terra system (Terra + Luna); no planets, so the planet
            // pipeline stays off.
            celestial_bodies: celestial.terra_system_bodies(),
            markers,
        }
    }
}

impl UIDrawable for IssAndHubbleSimulation {
    fn get_drawables(&mut self) -> Vec<UIDrawablePanel<'_>> {
        // The shared-core panel first (its callbacks borrow `self.simulation`),
        // then this scenario's own panel built from the disjoint
        // `self.last_telemetry` field (so it coexists with that borrow). The
        // readout loop is deliberately kept per-scenario (like the propagation
        // loop) - scenarios may diverge in how they present objects.
        let mut panels = self.simulation.get_drawables();
        let mut elements: Vec<Box<dyn Instrument>> =
            Vec::with_capacity(self.last_telemetry.len() * 3);
        for (index, sat) in self.last_telemetry.iter().enumerate() {
            let top = index as f32 * 74.0;
            elements.push(Box::new(Header {
                position: [0.0, top],
                title: sat.name.clone(),
            }));
            elements.push(Box::new(DualReadout {
                position: [0.0, top + 24.0],
                left_label: "Lat".to_string(),
                left_value: format!("{:.2} deg", sat.latitude_deg),
                right_label: "Lon".to_string(),
                right_value: format!("{:.2} deg", sat.longitude_deg),
            }));
            elements.push(Box::new(Readout {
                position: [0.0, top + 48.0],
                label: "Alt".to_string(),
                value: format!("{:.0} km", sat.altitude_km),
            }));
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

/// Builds the ISS + Hubble simulation and hands off to the winit event loop.
/// Blocks until the window closes.
pub fn run() {
    // Seed satkit's global state (embedded ephemeris + EOP table) before
    // anything else: IssAndHubbleSimulation::new below builds the
    // CelestialSphere (which reads the ephemeris) and the satellites parse
    // TLEs. Doing it here keeps satkit fully offline and data-dir-free.
    simulation::init();

    application::run(ApplicationState::new(IssAndHubbleSimulation::new()));
}
