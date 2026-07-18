//! ITRF (Earth-fixed) positions to WGS84 geodetic coordinates, delegating
//! to satkit.

use glam::DVec3;
use satkit::Vector3;
use satkit::itrfcoord::ITRFCoord;

/// WGS84 geodetic coordinates: radians, height above the ellipsoid meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geodetic {
    pub latitude_rad: f64,
    pub longitude_rad: f64,
    pub altitude_m: f64,
}

/// Geodetic coordinates of an ITRF position (meters).
pub fn geodetic_from_itrf(pos_itrf_m: DVec3) -> Geodetic {
    let coord = ITRFCoord::from_vector(&Vector3::new([
        [pos_itrf_m.x],
        [pos_itrf_m.y],
        [pos_itrf_m.z],
    ]));
    let (latitude_rad, longitude_rad, altitude_m) = coord.to_geodetic_rad();
    Geodetic {
        latitude_rad,
        longitude_rad,
        altitude_m,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WGS84 semi-major/semi-minor axes, meters.
    const WGS84_A_M: f64 = 6_378_137.0;
    const WGS84_B_M: f64 = 6_356_752.314_245;

    #[test]
    fn equator_prime_meridian() {
        let geo = geodetic_from_itrf(DVec3::new(WGS84_A_M + 1000.0, 0.0, 0.0));
        assert!(geo.latitude_rad.abs() < 1e-9);
        assert!(geo.longitude_rad.abs() < 1e-9);
        assert!((geo.altitude_m - 1000.0).abs() < 1e-3);
    }

    #[test]
    fn ninety_east_on_the_equator() {
        let geo = geodetic_from_itrf(DVec3::new(0.0, WGS84_A_M, 0.0));
        assert!((geo.longitude_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!(geo.altitude_m.abs() < 1e-3);
    }

    #[test]
    fn north_pole() {
        let geo = geodetic_from_itrf(DVec3::new(0.0, 0.0, WGS84_B_M + 500.0));
        assert!((geo.latitude_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!((geo.altitude_m - 500.0).abs() < 1e-3);
    }
}
