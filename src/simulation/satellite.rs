//! Satellite tracking: parse a TLE and propagate it with the satkit SGP4
//! implementation to a given datetime, exposing the result in the renderer's
//! world frame (km). Each [`Satellite`] is one tracked object; the simulation
//! holds a `Vec<Satellite>` (see `SimulationState`) and the array is assembled
//! in `main` from the inline TLE literals below. Only the TLE is retained; the
//! position state is a pure function of (TLE, datetime), so it is recomputed on
//! demand via `state_at` rather than stored - nothing in the struct goes stale
//! as the simulation clock advances.
//!
//! The flow is: TLE -> SGP4 (TEME, meters) -> rotate to ITRF/ECEF
//! (`qteme2itrf`) -> geodetic latitude/longitude/altitude (`ITRFCoord`) -> a
//! world-space point via the project's WGS84 helpers (`earth`), so the marker
//! lands on exactly the same ellipsoid the globe mesh is built from.
//!
//! `qteme2itrf` is the full (non-`approx`) transform: it reads satkit's global
//! EOP table (real polar motion + UT1-UTC), which `sky::init_satkit` pre-seeds
//! from the bundled `EOP-All.csv` at startup. That seeding also suppresses the
//! stray `satkit-data` dir satkit would otherwise create on first use; see its
//! docs.

use glam::Vec3;
use satkit::frametransform::qteme2itrf;
use satkit::itrfcoord::ITRFCoord;
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Instant, Vector3};

use crate::earth;

// Tracked-object TLEs, inlined as source literals. Unlike the
// textures/ephemeris/EOP (build-downloaded into the gitignored `assets/` and
// baked into the binary), these small element sets live directly in source so a
// fresh checkout needs no data file. The lines are column-sensitive TLE format
// (each element line is exactly 69 chars) - keep the exact spacing. `concat!`
// keeps source indentation out of the string. satkit parses by column and does
// not verify the trailing checksum digit. `main` assembles the tracked array
// from these via [`Satellite::from_tle`].

/// The International Space Station (ISS / ZARYA), epoch 2024-001.5. Real
/// element set.
pub const ISS_TLE: &str = concat!(
    "ISS (ZARYA)\n",
    "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9003\n",
    "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299357\n",
);

/// The Hubble Space Telescope (HST), epoch ~2024-001.5. NOTE: the orbit shape
/// is realistic (inclination 28.47 deg, ~540 km / ~15.1 rev/day), but this set
/// was assembled from memory - the RAAN/anomaly/epoch-fraction phase is
/// approximate. Replace with a freshly fetched TLE for true positional
/// accuracy. Included as a second tracked object so multiple markers render.
pub const HST_TLE: &str = concat!(
    "HST\n",
    "1 20580U 90037B   24001.49473380  .00002000  00000-0  10000-3 0  9990\n",
    "2 20580  28.4690  85.5400 0002600 310.0000  50.0000 15.09600000123456\n",
);

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
    /// Parses a 3-line TLE (name line + the two element lines, e.g. [`ISS_TLE`]).
    /// Panics on malformed input - the TLEs are inline source literals, so a
    /// failure is a build-time bug, handled like the other embedded data. No
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
    /// the origin, same axes as the globe mesh.
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
    let position_km = earth::surface_position(latitude, longitude)
        + earth::geodetic_normal(latitude, longitude) * altitude_km;

    SatelliteState {
        position_km,
        latitude_deg: latitude.to_degrees(),
        longitude_deg: longitude.to_degrees(),
        altitude_km,
    }
}
