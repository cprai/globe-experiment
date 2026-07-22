//! Crate-vs-reference geodetic-conversion comparisons.

/// The satkit reference side.
mod satkit {
    use engine_astrodynamics::geodetic::geodetic_from_itrf;
    use glam::DVec3;
    use satkit::itrfcoord::ITRFCoord;

    /// Same closed-form problem, same WGS84 constants on both sides - the
    /// bounds are near-machine (1e-9 rad ~ 6 mm on the surface).
    const ANGLE_TOL_RAD: f64 = 1e-9;
    const ALTITUDE_TOL_M: f64 = 1e-3;

    #[test]
    fn matches_satkit_over_grid() {
        crate::data::seed_satkit();
        for lat_deg in [-89.9, -60.0, -30.0, 0.0, 20.0, 45.0, 75.0, 89.9] {
            for lon_deg in [-179.0, -90.0, 0.0, 60.0, 135.0] {
                for alt_m in [-2_000.0_f64, 0.0, 400_000.0] {
                    let (lat, lon) = (f64::to_radians(lat_deg), f64::to_radians(lon_deg));
                    // Any test vector works as long as BOTH sides get the
                    // same one; spherical construction is fine.
                    let radius = 6_378_137.0 + alt_m;
                    let v = DVec3::new(
                        radius * lat.cos() * lon.cos(),
                        radius * lat.cos() * lon.sin(),
                        radius * lat.sin(),
                    );
                    let ours = geodetic_from_itrf(v);
                    let theirs =
                        ITRFCoord::from_vector(&satkit::Vector3::new([[v.x], [v.y], [v.z]]));
                    let (want_lat, want_lon, want_alt) = theirs.to_geodetic_rad();
                    let label = format!("({lat_deg}, {lon_deg}, {alt_m})");
                    assert!(
                        (ours.latitude_rad - want_lat).abs() < ANGLE_TOL_RAD,
                        "lat at {label}"
                    );
                    assert!(
                        (ours.longitude_rad - want_lon).abs() < ANGLE_TOL_RAD,
                        "lon at {label}"
                    );
                    assert!(
                        (ours.altitude_m - want_alt).abs() < ALTITUDE_TOL_M,
                        "alt at {label}: {} vs {want_alt}",
                        ours.altitude_m
                    );
                }
            }
        }
    }
}

/// The astrodyn (JEOD-port) reference side.
mod astrodyn {
    use astrodyn_math::GeodeticState;
    use engine_astrodynamics::geodetic::geodetic_from_itrf;
    use glam::DVec3;

    use crate::support::astrodyn_vec;

    /// WGS84 semi-axes handed to astrodyn's planet-agnostic entry point
    /// (the crate's constants are baked into its Vermeille closed form).
    const WGS84_A_M: f64 = 6_378_137.0;
    const WGS84_B_M: f64 = 6_356_752.314_245_179;

    /// Same closed-form problem, same ellipsoid on both sides (Vermeille
    /// vs JEOD's Borkowski iteration) - near-machine bounds, same as the
    /// satkit module.
    const ANGLE_TOL_RAD: f64 = 1e-9;
    const ALTITUDE_TOL_M: f64 = 1e-3;

    #[test]
    fn matches_astrodyn_over_grid() {
        for lat_deg in [-89.9, -60.0, -30.0, 0.0, 20.0, 45.0, 75.0, 89.9] {
            for lon_deg in [-179.0, -90.0, 0.0, 60.0, 135.0] {
                for alt_m in [-2_000.0_f64, 0.0, 400_000.0] {
                    let (lat, lon) = (f64::to_radians(lat_deg), f64::to_radians(lon_deg));
                    let radius = WGS84_A_M + alt_m;
                    let v = DVec3::new(
                        radius * lat.cos() * lon.cos(),
                        radius * lat.cos() * lon.sin(),
                        radius * lat.sin(),
                    );
                    let ours = geodetic_from_itrf(v);
                    let theirs =
                        GeodeticState::from_planet_fixed(astrodyn_vec(v), WGS84_A_M, WGS84_B_M);
                    let label = format!("({lat_deg}, {lon_deg}, {alt_m})");
                    assert!(
                        (ours.latitude_rad - theirs.latitude).abs() < ANGLE_TOL_RAD,
                        "lat at {label}: {} vs {}",
                        ours.latitude_rad,
                        theirs.latitude
                    );
                    assert!(
                        (ours.longitude_rad - theirs.longitude).abs() < ANGLE_TOL_RAD,
                        "lon at {label}: {} vs {}",
                        ours.longitude_rad,
                        theirs.longitude
                    );
                    assert!(
                        (ours.altitude_m - theirs.altitude).abs() < ALTITUDE_TOL_M,
                        "alt at {label}: {} vs {}",
                        ours.altitude_m,
                        theirs.altitude
                    );
                }
            }
        }
    }
}
