//! Space-station tracking: parse the embedded TLE and propagate it with the
//! satkit SGP4 implementation to a given datetime, exposing the result in the
//! renderer's world frame (km). The TLE is retained so the position can be
//! re-propagated each tick as the simulation clock advances.
//!
//! The flow is: TLE -> SGP4 (TEME, meters) -> rotate to ITRF/ECEF
//! (`qteme2itrf`) -> geodetic latitude/longitude/altitude (`ITRFCoord`) -> a
//! world-space point via the project's WGS84 helpers (`earth`), so the marker
//! lands on exactly the same ellipsoid the globe mesh is built from.
//!
//! `qteme2itrf` needs no data *file* (we run EOP-free, with zero polar motion),
//! but it - like every satkit frame transform - reads satkit's global EOP
//! table on first use, which lazily resolves a data dir and creates an empty
//! `satkit-data` dir as a side effect. `sky::init_satkit` pre-seeds that table
//! empty at startup to suppress the dir; see its docs.

use glam::Vec3;
use satkit::frametransform::qteme2itrf;
use satkit::itrfcoord::ITRFCoord;
use satkit::sgp4::sgp4;
use satkit::tle::TLE;
use satkit::{Instant, Vector3};

use super::earth;

/// The TLE for the station, embedded like the textures (assets/ is
/// gitignored, and everything else in the build is baked in too).
const TLE_TEXT: &str = include_str!("../../assets/TLE.txt");

/// A satellite tracked from its TLE, with its most recently propagated state.
pub struct Satellite {
    /// The parsed element set, kept for re-propagation as time advances.
    tle: TLE,
    /// Object name from the TLE's first line (e.g. "ISS (ZARYA)").
    pub name: String,
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

impl Satellite {
    /// Parses the embedded TLE and propagates it to its own epoch. Panics on
    /// malformed embedded data (the data is baked in, so a failure is a
    /// build-time bug, handled like the other embedded assets).
    pub fn load() -> Self {
        let mut lines = TLE_TEXT.lines();
        let line0 = lines.next().expect("TLE line 0 (name)");
        let line1 = lines.next().expect("TLE line 1");
        let line2 = lines.next().expect("TLE line 2");

        let mut tle = TLE::load_3line(line0, line1, line2).expect("parse embedded TLE");
        let name = tle.name.clone();
        let epoch = tle.epoch;

        let state = propagate(&mut tle, &epoch);
        Self {
            tle,
            name,
            position_km: state.position_km,
            latitude_deg: state.latitude_deg,
            longitude_deg: state.longitude_deg,
            altitude_km: state.altitude_km,
        }
    }

    /// The TLE's epoch - the simulation clock's natural starting time.
    pub fn epoch(&self) -> Instant {
        self.tle.epoch
    }

    /// Re-propagates the orbit to `time` and updates the stored state.
    pub fn update_to(&mut self, time: &Instant) {
        let state = propagate(&mut self.tle, time);
        self.position_km = state.position_km;
        self.latitude_deg = state.latitude_deg;
        self.longitude_deg = state.longitude_deg;
        self.altitude_km = state.altitude_km;
    }
}

/// The fields of `Satellite` that are recomputed on every propagation.
struct State {
    position_km: Vec3,
    latitude_deg: f32,
    longitude_deg: f32,
    altitude_km: f32,
}

/// SGP4-propagates `tle` to `time` and resolves the result to the world frame.
fn propagate(tle: &mut TLE, time: &Instant) -> State {
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

    State {
        position_km,
        latitude_deg: latitude.to_degrees(),
        longitude_deg: longitude.to_degrees(),
        altitude_km,
    }
}

impl Default for Satellite {
    fn default() -> Self {
        Self::load()
    }
}
