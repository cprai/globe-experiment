//! Satellite tracking: parse a TLE and propagate it with the satkit SGP4
//! implementation to a given datetime, exposing the result in the renderer's
//! world frame (km). Each [`Satellite`] is one tracked object; scenario structs
//! (see `crate::scenarios`) own a `Vec<Satellite>` assembled from that
//! scenario's own inline TLE literals (this module is element-set agnostic - it
//! propagates whatever TLEs a scenario hands it). Only the TLE is retained; the
//! position state is a pure function of (TLE, datetime), so it is recomputed on
//! demand via `state_at` rather than stored - nothing in the struct goes stale
//! as the simulation clock advances.
//!
//! The flow is: TLE -> SGP4 (TEME, meters) -> rotate to ITRF/ECEF
//! (`qteme2itrf`) -> geodetic latitude/longitude/altitude (`ITRFCoord`) -> a
//! world-space point via the project's WGS84 helpers (`terra`), so the marker
//! lands on exactly the same ellipsoid the Terra mesh is built from.
//!
//! Besides the single-time marker state, [`orbit_path_inertial`] propagates
//! one full period ahead for the renderer's predicted orbit path. It
//! dispatches on [`Propagation`]: analytic SGP4 from a TLE, or numerical
//! integration (satkit `orbitprop`) from a GCRF state vector ([`OrbitState`]).
//! The numerical arm needs no TLE, so a future manually-controlled satellite
//! can feed the same path renderer. See the function docs for the
//! deliberately different (inertial, single-rotation) frame treatment shared
//! by both arms.
//!
//! `qteme2itrf` is the full (non-`approx`) transform: it reads satkit's global
//! EOP table (real polar motion + UT1-UTC), which
//! `celestial_sphere::init_satkit` pre-seeds
//! from the bundled `EOP-All.csv` at startup. That seeding also suppresses the
//! stray `satkit-data` dir satkit would otherwise create on first use; see its
//! docs (the numerical arm's EGM96 gravity model is seeded the same way).

use glam::{DVec3, Vec3};
use satkit::frametransform::{qgcrf2itrf, qteme2gcrf, qteme2itrf};
use satkit::itrfcoord::ITRFCoord;
use satkit::orbitprop::{self, PropSettings, SimpleState};
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Duration, Instant, Kepler, Vector3};

use crate::terra;

/// An instantaneous orbital state vector in the GCRF frame: the initial
/// conditions for numerical propagation. Deliberately a plain-data type (no
/// satkit types) so a future manually-controlled satellite can construct one
/// directly, with no TLE behind it.
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
    /// Analytic SGP4 from the element set - cheap, for TLE-tracked objects.
    /// Boxed: a parsed `TLE` is ~1 KB (element strings + cached propagator
    /// init) vs the 48-byte state vector, and markers clone every frame.
    Sgp4(Box<TLE>),
    /// Numerical integration (satkit `orbitprop`) from GCRF initial
    /// conditions - works with no TLE (future manually-controlled
    /// satellites).
    Numerical(OrbitState),
}

/// A satellite tracked from its TLE. Holds only the (immutable-meaning) inputs:
/// the element set and the object name. The position state is derived on demand
/// from the TLE and a datetime - see [`SatelliteState`] and [`state_at`].
///
/// [`state_at`]: Satellite::state_at
pub struct Satellite {
    /// The parsed element set, propagated on demand for any requested time.
    /// `&mut` is needed to propagate it (satkit's `sgp4` caches its
    /// initialization in the TLE on the first call), so propagation methods
    /// take `&mut self`.
    tle: TLE,
    /// Object name from the TLE's first line (e.g. "ISS (ZARYA)").
    pub name: String,
}

impl Satellite {
    /// Parses a 3-line TLE (name line + the two element lines, e.g. a
    /// scenario's `ISS_TLE`). Panics on malformed input - the TLEs are
    /// inline source literals, so a failure is a build-time bug, handled
    /// like the other embedded data. No
    /// propagation happens here - the state is computed on demand via
    /// [`state_at`](Self::state_at).
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

    /// The parsed element set, for callers that carry it elsewhere (a
    /// scenario clones it into a marker's `Propagation::Sgp4` so the renderer
    /// can propagate the predicted orbit path itself).
    pub fn tle(&self) -> &TLE {
        &self.tle
    }

    /// Propagates the orbit to `time` and returns the resulting state in the
    /// world frame. Pure with respect to the satellite (nothing is stored);
    /// takes `&mut self` only because satkit's `sgp4` caches initialization in
    /// the TLE.
    pub fn state_at(&mut self, time: &Instant) -> SatelliteState {
        propagate(&mut self.tle, time)
    }
}

/// The satellite's propagated state at a particular time, derived from the TLE.
/// Recomputed on demand rather than stored on [`Satellite`].
pub struct SatelliteState {
    /// Position in the renderer's world frame: kilometers, planet center at
    /// the origin, same axes as the Terra mesh.
    pub position_km: Vec3,
    /// Sub-satellite geodetic latitude, degrees.
    pub latitude_deg: f32,
    /// Sub-satellite geodetic longitude, degrees.
    pub longitude_deg: f32,
    /// Height above the WGS84 ellipsoid, kilometers.
    pub altitude_km: f32,
    /// The GCRF state vector at the propagated time - initial conditions a
    /// scenario can hand to `Propagation::Numerical` for the predicted orbit
    /// path.
    pub orbit: OrbitState,
}

/// SGP4-propagates `tle` to `time` and resolves the result to the world frame.
fn propagate(tle: &mut TLE, time: &Instant) -> SatelliteState {
    // SGP4 -> position + velocity in the TEME frame, meters and m/s (one time
    // sample, so the 3xN matrices have a single column).
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

    // TEME -> GCRF state vector. Rotating the velocity by the same quaternion
    // as the position is correct: both frames are quasi-inertial, so there is
    // no omega-cross term (unlike a rotation into the Earth-fixed ITRF).
    let q_gcrf = qteme2gcrf(time);
    let pos_gcrf = q_gcrf * teme;
    let vel_gcrf = q_gcrf * teme_vel;

    // TEME -> ITRF (Earth-fixed), then to geodetic lat/lon/height.
    let itrf = qteme2itrf(time) * teme;
    let coord = ITRFCoord::from_vector(&itrf);
    let (lat_rad, lon_rad, hae_m) = coord.to_geodetic_rad();

    let latitude = lat_rad as f32;
    let longitude = lon_rad as f32;
    let altitude_km = (hae_m / 1000.0) as f32;

    // Reconstruct the world point from our own WGS84 helpers so the marker
    // sits on the exact ellipsoid the mesh uses: the surface point at (lat,
    // lon), raised along the geodetic normal by the altitude.
    let position_km = terra::surface_position(latitude, longitude)
        + terra::geodetic_normal(latitude, longitude) * altitude_km;

    SatelliteState {
        position_km,
        latitude_deg: latitude.to_degrees(),
        longitude_deg: longitude.to_degrees(),
        altitude_km,
        orbit: OrbitState {
            pos_gcrf_m: DVec3::new(pos_gcrf[0], pos_gcrf[1], pos_gcrf[2]),
            vel_gcrf_m_s: DVec3::new(vel_gcrf[0], vel_gcrf[1], vel_gcrf[2]),
        },
    }
}

/// Propagates one full orbital period ahead of `time` and returns
/// `segments + 1` world-frame sample points (km), the first at the object's
/// current position. Dispatches on the marker's [`Propagation`]: one batch
/// `sgp4` call, or one numerical `orbitprop` integration sampled through its
/// dense output - a scene may mix both kinds.
///
/// Frame treatment (shared by both arms) differs from the marker on purpose:
/// every inertial-frame sample is rotated into the Earth-fixed frame with the
/// SINGLE rotation at `time`, not each sample's own future rotation. That
/// renders the orbit as the star-fixed inertial ellipse - a closed curve that
/// Terra rotates under - rather than the open ground-track-like curve the
/// per-sample rotation would give. The path floats at orbital altitude, so no
/// geodetic round trip through the WGS84 helpers is needed (that exists on
/// the marker only to land it on the exact mesh ellipsoid); ITRF meters map
/// to world km by the axis permutation P alone (world (x,y,z) = ITRF (y,z,x),
/// see `coordinates.md`).
pub fn orbit_path_inertial(prop: &Propagation, time: &Instant, segments: usize) -> Vec<Vec3> {
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

/// ITRF meters -> world km: the axis permutation P (world (x,y,z) =
/// ITRF (y,z,x)) plus the unit change. See `coordinates.md`.
fn world_km_from_itrf_m(itrf: &Vector3) -> Vec3 {
    Vec3::new(
        (itrf[1] / 1000.0) as f32,
        (itrf[2] / 1000.0) as f32,
        (itrf[0] / 1000.0) as f32,
    )
}

/// The SGP4 path arm: one batch call over the TLE, period from the element
/// set's mean motion.
fn orbit_path_sgp4(tle: &TLE, time: &Instant, segments: usize) -> Vec<Vec3> {
    // `sgp4` needs `&mut` (it caches its propagator init in the TLE), but the
    // caller's TLE sits behind a shared `RenderState` borrow - clone locally.
    let mut tle = tle.clone();

    // TLE mean motion is revolutions per day, so one period in seconds:
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

/// The numerical path arm: satkit's `orbitprop` integrator from the GCRF
/// initial conditions - no TLE involved. One `propagate` over the period,
/// then all samples from its dense output in one `interp_batch`.
fn orbit_path_numerical(state: &OrbitState, time: &Instant, segments: usize) -> Vec<Vec3> {
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

    // Period from the osculating elements (depends only on the semi-major
    // axis, so circular/equatorial angle singularities cannot bite). Errs
    // only for a non-elliptic (e >= 1) state; today every state derives from
    // a sane embedded TLE, so failure is a build bug - when
    // manually-controlled satellites arrive, convert this (and the propagate
    // expects below) to an empty-path fallback instead.
    let period_s = Kepler::from_pv(pos, vel)
        .expect("elliptic orbit state for path period")
        .period();

    let mut initial = SimpleState::zeros();
    initial[0] = state.pos_gcrf_m.x;
    initial[1] = state.pos_gcrf_m.y;
    initial[2] = state.pos_gcrf_m.z;
    initial[3] = state.vel_gcrf_m_s.x;
    initial[4] = state.vel_gcrf_m_s.y;
    initial[5] = state.vel_gcrf_m_s.z;

    // Defaults: EGM96 gravity 4x4 (seeded from embedded bytes in
    // `init_satkit`), Sun/Moon third-body (embedded DE440), solid tides,
    // relativistic correction, adaptive RKV98 with dense output. Drag and
    // solar radiation pressure only run when `satprops` is Some, so passing
    // None below keeps satkit's space-weather loader (a satkit-data file this
    // build does not embed) unreachable; `use_spaceweather: false` is
    // belt-and-suspenders for the same reason.
    let settings = PropSettings {
        use_spaceweather: false,
        ..PropSettings::default()
    };
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
