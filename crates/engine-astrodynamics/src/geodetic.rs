//! ITRF (Earth-fixed) positions to WGS84 geodetic coordinates - crate-owned
//! closed form (Vermeille 2002). Deliberately NOT anise's `latlongalt()`:
//! that uses the planetary-constants ellipsoid (a = 6 378 136.6 m, IAU),
//! while this crate's contract and the engine's `planet.rs` are WGS84
//! (a = 6 378 137 m).

use glam::DVec3;

/// WGS84 semi-major axis, meters.
const WGS84_A_M: f64 = 6_378_137.0;
/// WGS84 inverse flattening.
const WGS84_INV_F: f64 = 298.257_223_563;

/// WGS84 geodetic coordinates: radians, height above the ellipsoid meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geodetic {
    pub latitude_rad: f64,
    pub longitude_rad: f64,
    pub altitude_m: f64,
}

/// Geodetic coordinates of an ITRF position (meters), via Vermeille's
/// non-iterative solution (valid everywhere the app can point, including
/// below the surface). Total function: every finite input maps to a
/// coordinate triple.
pub fn geodetic_from_itrf(pos_itrf_m: DVec3) -> Geodetic {
    let f = 1.0 / WGS84_INV_F;
    let e2 = f * (2.0 - f);
    let e4 = e2 * e2;

    let (x, y, z) = (pos_itrf_m.x, pos_itrf_m.y, pos_itrf_m.z);
    let p = (x * x + y * y) / (WGS84_A_M * WGS84_A_M);
    let q = (1.0 - e2) * z * z / (WGS84_A_M * WGS84_A_M);
    let r = (p + q - e4) / 6.0;
    let s = e4 * p * q / (4.0 * r * r * r);
    let t = (1.0 + s + (s * (2.0 + s)).sqrt()).cbrt();
    let u = r * (1.0 + t + 1.0 / t);
    let v = (u * u + e4 * q).sqrt();
    let w = e2 * (u + v - q) / (2.0 * v);
    let k = (u + v + w * w).sqrt() - w;
    let d = k * (x * x + y * y).sqrt() / (k + e2);

    let hypot_dz = (d * d + z * z).sqrt();
    Geodetic {
        latitude_rad: 2.0 * z.atan2(d + hypot_dz),
        longitude_rad: y.atan2(x),
        altitude_m: (k + e2 - 1.0) / k * hypot_dz,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WGS84 semi-minor axis, meters.
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

    /// The exact forward (geodetic -> ITRF) map: prime-vertical radius
    /// N(lat), textbook closed form. Trivially correct, which makes the
    /// round trip a real check of the inverse above.
    fn itrf_from_geodetic(latitude_rad: f64, longitude_rad: f64, altitude_m: f64) -> DVec3 {
        let f = 1.0 / WGS84_INV_F;
        let e2 = f * (2.0 - f);
        let n = WGS84_A_M / (1.0 - e2 * latitude_rad.sin().powi(2)).sqrt();
        DVec3::new(
            (n + altitude_m) * latitude_rad.cos() * longitude_rad.cos(),
            (n + altitude_m) * latitude_rad.cos() * longitude_rad.sin(),
            (n * (1.0 - e2) + altitude_m) * latitude_rad.sin(),
        )
    }

    /// Round trip over a lat/lon/alt grid (including below the ellipsoid)
    /// at sub-mm / sub-nanoradian agreement.
    #[test]
    fn forward_inverse_round_trip() {
        for lat_deg in [-89.5, -60.0, -30.0, 0.0, 20.0, 45.0, 75.0, 89.5] {
            for lon_deg in [-179.0, -90.0, 0.0, 60.0, 135.0] {
                for alt_m in [-2_000.0, 0.0, 8_848.0, 400_000.0] {
                    let (lat, lon) = (f64::to_radians(lat_deg), f64::to_radians(lon_deg));
                    let geo = geodetic_from_itrf(itrf_from_geodetic(lat, lon, alt_m));
                    assert!(
                        (geo.latitude_rad - lat).abs() < 1e-11,
                        "lat at ({lat_deg}, {lon_deg}, {alt_m})"
                    );
                    assert!(
                        (geo.longitude_rad - lon).abs() < 1e-11,
                        "lon at ({lat_deg}, {lon_deg}, {alt_m})"
                    );
                    assert!(
                        (geo.altitude_m - alt_m).abs() < 1e-4,
                        "alt at ({lat_deg}, {lon_deg}, {alt_m}): {}",
                        geo.altitude_m
                    );
                }
            }
        }
    }
}
