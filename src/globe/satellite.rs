//! Space-station tracking: parse the embedded TLE, propagate it with the
//! satkit SGP4 implementation to a fixed datetime, and expose the result in
//! the renderer's world frame (km).
//!
//! The flow is: TLE -> SGP4 (TEME, meters) -> rotate to ITRF/ECEF
//! (`qteme2itrf`, a GMST-only rotation that needs no downloaded data files)
//! -> geodetic latitude/longitude/altitude (`ITRFCoord`) -> a world-space
//! point via the project's WGS84 helpers (`earth`), so the marker lands on
//! exactly the same ellipsoid the globe mesh is built from.

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

/// The fixed UTC datetime the position is evaluated at: the TLE's own epoch
/// (`24001.50000000` = 2024 day-of-year 1.5). TLEs are only accurate for a
/// few days around their epoch, so we pin the evaluation here. Shown in the
/// UI as a read-only label.
const EVAL_LABEL: &str = "2024-01-01 12:00:00 UTC";

/// A propagated satellite position, ready to render and to display.
pub struct Satellite {
    /// Object name from the TLE's first line (e.g. "ISS (ZARYA)").
    pub name: String,
    /// The datetime the position was computed for (read-only display).
    pub time_label: &'static str,
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
    /// Parses the embedded TLE and propagates it to the fixed eval time.
    /// Panics on malformed embedded data (the data is baked in, so a failure
    /// is a build-time bug, handled like the other embedded assets).
    pub fn load() -> Self {
        let mut lines = TLE_TEXT.lines();
        let line0 = lines.next().expect("TLE line 0 (name)");
        let line1 = lines.next().expect("TLE line 1");
        let line2 = lines.next().expect("TLE line 2");

        let mut tle = TLE::load_3line(line0, line1, line2).expect("parse embedded TLE");

        let time = Instant::from_datetime(2024, 1, 1, 12, 0, 0.0).expect("build eval instant");

        // SGP4 -> position in the TEME frame, meters (one time sample, so the
        // 3xN position matrix has a single column).
        let state = sgp4(&mut tle, &[time]).expect("sgp4 propagation");
        let teme = Vector3::new([
            [state.pos[(0, 0)]],
            [state.pos[(1, 0)]],
            [state.pos[(2, 0)]],
        ]);

        // TEME -> ITRF (Earth-fixed), then to geodetic lat/lon/height.
        let itrf = qteme2itrf(&time) * teme;
        let coord = ITRFCoord::from_vector(&itrf);
        let (lat_rad, lon_rad, hae_m) = coord.to_geodetic_rad();

        let latitude = lat_rad as f32;
        let longitude = lon_rad as f32;
        let altitude_km = (hae_m / 1000.0) as f32;

        // Reconstruct the world point from our own WGS84 helpers so the
        // marker sits on the exact ellipsoid the mesh uses: the surface point
        // at (lat, lon), raised along the geodetic normal by the altitude.
        let position_km = earth::surface_position(latitude, longitude)
            + earth::geodetic_normal(latitude, longitude) * altitude_km;

        Self {
            name: tle.name.clone(),
            time_label: EVAL_LABEL,
            position_km,
            latitude_deg: latitude.to_degrees(),
            longitude_deg: longitude.to_degrees(),
            altitude_km,
        }
    }
}

impl Default for Satellite {
    fn default() -> Self {
        Self::load()
    }
}
