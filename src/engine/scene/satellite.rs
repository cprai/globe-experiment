//! Satellite tracking: TLE parse + satkit SGP4, plus a TLE-free numerical
//! pipeline, resolved into the renderer's world frame (km). TLE literals
//! live in the scenes; this module propagates whatever it is handed.
//! Position is never stored - it is a pure function of (elements, time),
//! recomputed on demand, so nothing goes stale as the clock advances.
//!
//! Marker chain: SGP4 (TEME, m) -> ITRF via the full `qteme2itrf` (reads the
//! EOP table pre-seeded in `celestial_sphere::init_satkit`) -> geodetic ->
//! world via the project WGS84 helpers, so the marker lands on exactly the
//! ellipsoid the Terra impostor traces.
//!
//! [`orbit_path_inertial`] propagates one period ahead for the predicted
//! orbit path, dispatching on [`Propagation`] (analytic SGP4 or numerical
//! `orbitprop`); see its doc for the deliberate single-rotation inertial
//! frame treatment. TLE-free manual-control helpers:
//! [`propagate_numerical`], [`resolve_orbit`], [`orbit_shape`].

use glam::DVec3;
use pyo3::prelude::*;
use satkit::frametransform::{qgcrf2itrf, qteme2gcrf, qteme2itrf};
use satkit::itrfcoord::ITRFCoord;
use satkit::orbitprop::{self, PropSettings, SimpleState};
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Duration, Instant, Kepler, Vector3};

use crate::engine::planet;
use crate::engine::scene::body::CelestialBody;

/// An instantaneous GCRF orbital state vector - numerical-propagation
/// initial conditions. Deliberately plain data (no satkit types) so a
/// manually-controlled satellite can construct one with no TLE behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitState {
    /// Position, GCRF, meters.
    pub pos_gcrf_m: DVec3,
    /// Velocity, GCRF, meters/second.
    pub vel_gcrf_m_s: DVec3,
}

/// How the renderer predicts an object's future orbit path. Carried on each
/// `SatelliteMarker`; a scene may mix both kinds.
#[derive(Clone, Debug)]
pub enum Propagation {
    /// Analytic SGP4 from the element set. Boxed: a parsed `TLE` is ~1 KB
    /// vs the 48-byte state vector, and markers clone every frame.
    Sgp4(Box<TLE>),
    /// Numerical `orbitprop` from GCRF initial conditions - no TLE needed.
    Numerical(OrbitState),
}

/// A satellite tracked from its TLE. Holds only the element set and name;
/// state is derived on demand via [`state_at`](Self::state_at).
pub struct Satellite {
    /// `&mut` is needed to propagate (satkit's `sgp4` caches its
    /// initialization in the TLE on first call), so propagation methods take
    /// `&mut self`.
    tle: TLE,
    /// Object name from the TLE (e.g. "ISS (ZARYA)").
    pub name: String,
}

impl Satellite {
    /// Parses a 3-line TLE (name line + two element lines). Panics on
    /// malformed input - TLEs are inline source literals, so a failure is a
    /// build-time bug, handled like the other embedded data.
    pub fn from_tle(tle_3line: &str) -> Self {
        let mut lines = tle_3line.lines();
        let line0 = lines.next().expect("TLE line 0 (name)");
        let line1 = lines.next().expect("TLE line 1");
        let line2 = lines.next().expect("TLE line 2");

        let tle = TLE::load_3line(line0, line1, line2).expect("parse embedded TLE");
        let name = tle.name.clone();
        Self { tle, name }
    }

    /// The TLE's epoch - the simulation clock's natural starting time.
    pub fn epoch(&self) -> Instant {
        self.tle.epoch
    }

    /// The parsed element set.
    pub fn tle(&self) -> &TLE {
        &self.tle
    }

    /// Propagates to `time`, in the world frame. `&mut self` only because
    /// satkit's `sgp4` caches initialization in the TLE; nothing is stored.
    pub fn state_at(&mut self, time: &Instant) -> SatelliteState {
        propagate(&mut self.tle, time)
    }
}

/// The propagated state at one time, recomputed on demand.
pub struct SatelliteState {
    /// Position in the renderer's world frame: km, planet center at the
    /// origin, same axes as the Terra body frame.
    pub position_km: DVec3,
    /// Sub-satellite geodetic latitude, degrees.
    pub latitude_deg: f64,
    /// Sub-satellite geodetic longitude, degrees.
    pub longitude_deg: f64,
    /// Height above the WGS84 ellipsoid, kilometers.
    pub altitude_km: f64,
    /// The GCRF state vector at the propagated time - initial conditions
    /// for `Propagation::Numerical`.
    pub orbit: OrbitState,
}

/// SGP4-propagates `tle` to `time` and resolves the result to the world frame.
fn propagate(tle: &mut TLE, time: &Instant) -> SatelliteState {
    // SGP4 -> TEME position + velocity (m, m/s); one time sample, so the
    // 3xN matrices have a single column.
    let sgp4_state = sgp4(tle, &[*time]).expect("sgp4 propagation");
    let teme = Vector3::new([
        [sgp4_state.pos[(0, 0)]],
        [sgp4_state.pos[(1, 0)]],
        [sgp4_state.pos[(2, 0)]],
    ]);
    let teme_vel = Vector3::new([
        [sgp4_state.vel[(0, 0)]],
        [sgp4_state.vel[(1, 0)]],
        [sgp4_state.vel[(2, 0)]],
    ]);

    // TEME -> GCRF. Rotating the velocity by the same quaternion as the
    // position is correct: both frames are quasi-inertial, so there is no
    // omega-cross term (unlike a rotation into the Earth-fixed ITRF).
    let q_gcrf = qteme2gcrf(time);
    let pos_gcrf = q_gcrf * teme;
    let vel_gcrf = q_gcrf * teme_vel;

    // TEME -> ITRF (Earth-fixed), then to geodetic lat/lon/height.
    let itrf = qteme2itrf(time) * teme;

    state_from_itrf(
        &itrf,
        OrbitState {
            pos_gcrf_m: DVec3::new(pos_gcrf[0], pos_gcrf[1], pos_gcrf[2]),
            vel_gcrf_m_s: DVec3::new(vel_gcrf[0], vel_gcrf[1], vel_gcrf[2]),
        },
    )
}

/// Resolves a GCRF state vector to the same [`SatelliteState`] the SGP4 arm
/// produces - a pure frame change; the state must already be propagated to
/// `time`.
pub fn resolve_orbit(state: &OrbitState, time: &Instant) -> SatelliteState {
    let gcrf = Vector3::new([
        [state.pos_gcrf_m.x],
        [state.pos_gcrf_m.y],
        [state.pos_gcrf_m.z],
    ]);
    let itrf = qgcrf2itrf(time) * gcrf;
    state_from_itrf(&itrf, *state)
}

/// Shared Earth-fixed tail: ITRF meters -> geodetic -> a world point rebuilt
/// from our own WGS84 helpers (surface point + geodetic normal * altitude).
/// The geodetic round trip is deliberate: it lands the marker on the exact
/// ellipsoid the impostor traces. `orbit` passes through untouched.
fn state_from_itrf(itrf: &Vector3, orbit: OrbitState) -> SatelliteState {
    let coord = ITRFCoord::from_vector(itrf);
    let (lat_rad, lon_rad, hae_m) = coord.to_geodetic_rad();

    let altitude_km = hae_m / 1000.0;

    let position_km = planet::surface_position(CelestialBody::TERRA, lat_rad, lon_rad)
        + planet::geodetic_normal(CelestialBody::TERRA, lat_rad, lon_rad) * altitude_km;

    SatelliteState {
        position_km,
        latitude_deg: lat_rad.to_degrees(),
        longitude_deg: lon_rad.to_degrees(),
        altitude_km,
        orbit,
    }
}

/// The [`OrbitState`] packed as satkit's 6-vector integrator state
/// (GCRF x,y,z meters + vx,vy,vz m/s).
fn simple_state(state: &OrbitState) -> SimpleState {
    let mut packed = SimpleState::zeros();
    packed[0] = state.pos_gcrf_m.x;
    packed[1] = state.pos_gcrf_m.y;
    packed[2] = state.pos_gcrf_m.z;
    packed[3] = state.vel_gcrf_m_s.x;
    packed[4] = state.vel_gcrf_m_s.y;
    packed[5] = state.vel_gcrf_m_s.z;
    packed
}

/// Shared `orbitprop` settings: defaults (EGM96 4x4, Sun/Moon third-body,
/// solid tides, relativity, adaptive RKV98 dense output). Drag/SRP only run
/// when `propagate`'s `satprops` is Some - every caller here passes None to
/// keep satkit's non-embedded space-weather loader unreachable;
/// `use_spaceweather: false` is belt-and-suspenders for the same reason.
fn numerical_settings() -> PropSettings {
    PropSettings {
        use_spaceweather: false,
        ..PropSettings::default()
    }
}

/// Numerically steps a GCRF state from `from` to `to` (one `orbitprop`
/// integration). The manual-control per-frame re-anchor: the scene stores
/// the result as its new initial conditions at `to`, so a burn's velocity
/// change compounds into every later frame.
pub fn propagate_numerical(state: &OrbitState, from: &Instant, to: &Instant) -> OrbitState {
    let initial = simple_state(state);
    let result = orbitprop::propagate(&initial, from, to, &numerical_settings(), None)
        .expect("numerical state propagation");
    let end = result.state_end;
    OrbitState {
        pos_gcrf_m: DVec3::new(end[0], end[1], end[2]),
        vel_gcrf_m_s: DVec3::new(end[3], end[4], end[5]),
    }
}

/// Osculating-orbit panel readout: apsis altitudes + current speed.
/// `pyclass` (`get_all`) so a `*_py` scene's script reads the same readouts
/// its Rust sibling formats.
#[pyclass(module = "globe", get_all)]
pub struct OrbitShape {
    /// Apoapsis height above Terra's mean radius, km.
    pub apoapsis_alt_km: f64,
    /// Periapsis height above Terra's mean radius, km.
    pub periapsis_alt_km: f64,
    /// Current inertial speed, m/s.
    pub speed_m_s: f64,
}

/// Osculating apsides + speed from a GCRF state vector. Apsis radii from the
/// Keplerian `a`/`e` (`r = a(1 +/- e)`); altitudes are above the *mean*
/// radius (a spherical convenience readout, not the marker's geodetic WGS84
/// altitude). `None` for a non-elliptic (e >= 1, escape) state - no apoapsis
/// exists; same fallback as the path renderer's empty path.
pub fn orbit_shape(state: &OrbitState) -> Option<OrbitShape> {
    let pos = Vector3::new([
        [state.pos_gcrf_m.x],
        [state.pos_gcrf_m.y],
        [state.pos_gcrf_m.z],
    ]);
    let vel = Vector3::new([
        [state.vel_gcrf_m_s.x],
        [state.vel_gcrf_m_s.y],
        [state.vel_gcrf_m_s.z],
    ]);
    let kepler = Kepler::from_pv(pos, vel).ok()?;
    let mean_radius_m = planet::TERRA_MEAN_RADIUS_KM * 1000.0;
    Some(OrbitShape {
        apoapsis_alt_km: (kepler.a * (1.0 + kepler.eccen) - mean_radius_m) / 1000.0,
        periapsis_alt_km: (kepler.a * (1.0 - kepler.eccen) - mean_radius_m) / 1000.0,
        speed_m_s: state.vel_gcrf_m_s.length(),
    })
}

/// Propagates one full period ahead of `time`, returning `segments + 1`
/// world-frame samples (km), the first at the current position. The
/// numerical arm returns an EMPTY vector for a non-elliptic (escape) state,
/// which has no period; the renderer skips such a path.
///
/// Frame treatment (both arms) deliberately differs from the marker: every
/// inertial sample is rotated Earth-fixed with the SINGLE rotation at
/// `time`, not each sample's own future rotation - rendering the star-fixed
/// inertial ellipse (a closed curve Terra rotates under), not a ground
/// track. The path floats at altitude, so no geodetic round trip (that
/// exists on the marker only); ITRF m -> world km is the plain permutation P
/// (see `coordinates.md`).
pub fn orbit_path_inertial(prop: &Propagation, time: &Instant, segments: usize) -> Vec<DVec3> {
    match prop {
        Propagation::Sgp4(tle) => orbit_path_sgp4(tle, time, segments),
        Propagation::Numerical(state) => orbit_path_numerical(state, time, segments),
    }
}

/// The `segments + 1` sample instants spanning one period from `time`.
fn path_sample_times(time: &Instant, period_s: f64, segments: usize) -> Vec<Instant> {
    (0..=segments)
        .map(|i| *time + Duration::from_seconds(period_s * i as f64 / segments as f64))
        .collect()
}

/// ITRF meters -> world km: the permutation P (world (x,y,z) = ITRF (y,z,x))
/// plus the unit change.
fn world_km_from_itrf_m(itrf: &Vector3) -> DVec3 {
    DVec3::new(itrf[1] / 1000.0, itrf[2] / 1000.0, itrf[0] / 1000.0)
}

/// SGP4 path arm: one batch call; period from the element set's mean motion.
fn orbit_path_sgp4(tle: &TLE, time: &Instant, segments: usize) -> Vec<DVec3> {
    // `sgp4` needs `&mut` (it caches its propagator init in the TLE), but
    // the caller's TLE sits behind a shared borrow - clone locally.
    let mut tle = tle.clone();

    // TLE mean motion is revolutions per day.
    let period_s = 86_400.0 / tle.mean_motion;
    let times = path_sample_times(time, period_s, segments);
    let state = sgp4(&mut tle, &times).expect("sgp4 orbit path propagation");

    let q = qteme2itrf(time);
    (0..=segments)
        .map(|i| {
            let teme = Vector3::new([
                [state.pos[(0, i)]],
                [state.pos[(1, i)]],
                [state.pos[(2, i)]],
            ]);
            world_km_from_itrf_m(&(q * teme))
        })
        .collect()
}

/// Numerical path arm: one `orbitprop` propagate over the period, all
/// samples from its dense output in one `interp_batch`.
fn orbit_path_numerical(state: &OrbitState, time: &Instant, segments: usize) -> Vec<DVec3> {
    let pos = Vector3::new([
        [state.pos_gcrf_m.x],
        [state.pos_gcrf_m.y],
        [state.pos_gcrf_m.z],
    ]);
    let vel = Vector3::new([
        [state.vel_gcrf_m_s.x],
        [state.vel_gcrf_m_s.y],
        [state.vel_gcrf_m_s.z],
    ]);

    // Period from the osculating elements depends only on the semi-major
    // axis, so circular/equatorial singularities cannot bite. e >= 1
    // (escape, reachable by burning) has no period - return the empty path
    // (the renderer skips it) rather than panic.
    let Ok(kepler) = Kepler::from_pv(pos, vel) else {
        return Vec::new();
    };
    let period_s = kepler.period();

    let initial = simple_state(state);
    let settings = numerical_settings();
    let end = *time + Duration::from_seconds(period_s);
    let result = orbitprop::propagate(&initial, time, &end, &settings, None)
        .expect("numerical orbit path propagation");

    let times = path_sample_times(time, period_s, segments);
    let samples = result
        .interp_batch(&times)
        .expect("orbit path dense-output sampling");

    let q = qgcrf2itrf(time);
    samples
        .iter()
        .map(|sample| {
            let gcrf = Vector3::new([[sample[0]], [sample[1]], [sample[2]]]);
            world_km_from_itrf_m(&(q * gcrf))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terra's GM (EGM96, m^3/s^2) - only to construct the test state's
    /// circular speed; the propagator brings its own force model.
    const MU_M3_S2: f64 = 3.986004418e14;
    /// Test orbit radius, meters (~407 km above the mean radius).
    const RADIUS_M: f64 = 6_778_000.0;

    /// A circular equatorial LEO state vector with no TLE behind it - the
    /// same construction a manually-controlled satellite lives on.
    fn circular_leo() -> (OrbitState, Instant) {
        let speed = (MU_M3_S2 / RADIUS_M).sqrt();
        let state = OrbitState {
            pos_gcrf_m: DVec3::new(RADIUS_M, 0.0, 0.0),
            vel_gcrf_m_s: DVec3::new(0.0, speed, 0.0),
        };
        let time = Instant::from_datetime(2024, 1, 1, 12, 0, 0.0).expect("valid datetime");
        (state, time)
    }

    /// The TLE-free pipeline must hold a circular LEO: shape readout matches
    /// the constructed altitude/speed, ten minutes of propagation stays on
    /// the (near-)circle while moving along track, and the resolved marker
    /// lands on the orbit radius near the equator. Loose km-scale tolerances
    /// absorb the full force model (J2 & co.) vs the two-body construction;
    /// a frame or unit mix-up misses by orders of magnitude.
    #[test]
    fn numerical_pipeline_holds_circular_leo() {
        crate::engine::scene::celestial_sphere::init_satkit_for_tests();

        let (state, t0) = circular_leo();
        let alt_km = (RADIUS_M - planet::TERRA_MEAN_RADIUS_KM * 1000.0) / 1000.0;

        let shape = orbit_shape(&state).expect("circular orbit is elliptic");
        assert!(
            (shape.apoapsis_alt_km - alt_km).abs() < 5.0,
            "apoapsis {:.1} km, expected ~{alt_km:.1}",
            shape.apoapsis_alt_km
        );
        assert!(
            (shape.periapsis_alt_km - alt_km).abs() < 5.0,
            "periapsis {:.1} km, expected ~{alt_km:.1}",
            shape.periapsis_alt_km
        );
        assert!(
            (shape.speed_m_s - state.vel_gcrf_m_s.length()).abs() < 1e-6,
            "speed readout is the state's own speed"
        );

        let t1 = t0 + Duration::from_seconds(600.0);
        let stepped = propagate_numerical(&state, &t0, &t1);
        assert!(
            (stepped.pos_gcrf_m.length() - RADIUS_M).abs() < 30_000.0,
            "radius drifted to {:.1} km",
            stepped.pos_gcrf_m.length() / 1000.0
        );
        assert!(
            (stepped.vel_gcrf_m_s.length() - shape.speed_m_s).abs() < 50.0,
            "speed drifted to {:.1} m/s",
            stepped.vel_gcrf_m_s.length()
        );
        assert!(
            (stepped.pos_gcrf_m - state.pos_gcrf_m).length() > 1_000_000.0,
            "propagation should move well along track in 600 s"
        );

        let resolved = resolve_orbit(&stepped, &t1);
        assert!(
            (resolved.position_km.length() * 1000.0 - RADIUS_M).abs() < 30_000.0,
            "marker at {:.1} km from center",
            resolved.position_km.length()
        );
        assert!(
            resolved.latitude_deg.abs() < 1.0,
            "equatorial orbit resolved to lat {:.2}",
            resolved.latitude_deg
        );
    }
}
