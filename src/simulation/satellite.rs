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
//! Besides the single-time marker state, [`orbit_path_inertial`] batch-
//! propagates a TLE one full period ahead for the renderer's predicted orbit
//! path - see its docs for the deliberately different (inertial, single-
//! rotation) frame treatment.
//!
//! `qteme2itrf` is the full (non-`approx`) transform: it reads satkit's global
//! EOP table (real polar motion + UT1-UTC), which
//! `celestial_sphere::init_satkit` pre-seeds
//! from the bundled `EOP-All.csv` at startup. That seeding also suppresses the
//! stray `satkit-data` dir satkit would otherwise create on first use; see its
//! docs.

use glam::Vec3;
use satkit::frametransform::qteme2itrf;
use satkit::itrfcoord::ITRFCoord;
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Duration, Instant, Vector3};

use crate::terra;

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

    /// The parsed element set, for callers that carry it elsewhere (the
    /// scenario clones it into each frame's `SatelliteMarker` so the renderer
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
}

/// SGP4-propagates `tle` to `time` and resolves the result to the world frame.
fn propagate(tle: &mut TLE, time: &Instant) -> SatelliteState {
    // SGP4 -> position in the TEME frame, meters (one time sample, so the 3xN
    // position matrix has a single column).
    let sgp4_state = sgp4(tle, &[*time]).expect("sgp4 propagation");
    let teme = Vector3::new([
        [sgp4_state.pos[(0, 0)]],
        [sgp4_state.pos[(1, 0)]],
        [sgp4_state.pos[(2, 0)]],
    ]);

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
    }
}

/// SGP4-propagates `tle` one full orbital period ahead of `time` and returns
/// `segments + 1` world-frame sample points (km), the first at the satellite's
/// current position. One batch `sgp4` call covers the whole path.
///
/// Frame treatment differs from the marker on purpose: every TEME sample is
/// rotated into the Earth-fixed frame with the SINGLE rotation at `time`
/// (`qteme2itrf(time)`), not each sample's own future rotation. That renders
/// the orbit as the star-fixed inertial ellipse - a closed curve that Terra
/// rotates under - rather than the open ground-track-like curve the per-sample
/// rotation would give. The path floats at orbital altitude, so no geodetic
/// round trip through the WGS84 helpers is needed (that exists on the marker
/// only to land it on the exact mesh ellipsoid); ITRF meters map to world km
/// by the axis permutation P alone (world (x,y,z) = ITRF (y,z,x), see
/// `coordinates.md`).
pub fn orbit_path_inertial(tle: &TLE, time: &Instant, segments: usize) -> Vec<Vec3> {
    // `sgp4` needs `&mut` (it caches its propagator init in the TLE), but the
    // caller's TLE sits behind a shared `RenderState` borrow - clone locally.
    let mut tle = tle.clone();

    // TLE mean motion is revolutions per day, so one period in seconds:
    let period_s = 86_400.0 / tle.mean_motion;
    let times: Vec<Instant> = (0..=segments)
        .map(|i| *time + Duration::from_seconds(period_s * i as f64 / segments as f64))
        .collect();
    let state = sgp4(&mut tle, &times).expect("sgp4 orbit path propagation");

    let q = qteme2itrf(time);
    (0..=segments)
        .map(|i| {
            let teme = Vector3::new([
                [state.pos[(0, i)]],
                [state.pos[(1, i)]],
                [state.pos[(2, i)]],
            ]);
            let itrf = q * teme;
            Vec3::new(
                (itrf[1] / 1000.0) as f32,
                (itrf[2] / 1000.0) as f32,
                (itrf[0] / 1000.0) as f32,
            )
        })
        .collect()
}
