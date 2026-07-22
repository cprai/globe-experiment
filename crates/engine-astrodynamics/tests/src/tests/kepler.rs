//! Crate-vs-reference Kepler-element comparisons.

/// The satkit reference side.
mod satkit {
    use engine_astrodynamics::kepler::Kepler;
    use glam::DVec3;

    /// Documented deviation from the plan's 1e-9: the crate's mu comes from
    /// the planetary-constants kernel (DE440 value, 398600.4354 km^3/s^2)
    /// while satkit uses its own constant (WGS84-class 398600.4418) - a
    /// ~1.6e-8 relative gap. At a FIXED perigee state that gap amplifies
    /// into the elements with eccentricity: da/a = (1+e)/(1-e) x dmu/mu
    /// (from 1/a = 2/r - v^2/mu), and dT/T = 1.5 da/a - 0.5 dmu/mu - x199
    /// at e = 0.99. Bounds therefore scale per state (3x headroom over the
    /// mu floor); measured agreement confirms the attribution (e.g.
    /// 1.44e-7 on the e = 0.7 period vs 1.29e-7 predicted).
    const MU_PROVENANCE_REL: f64 = 1.61e-8;

    fn sma_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity) / (1.0 - eccentricity)
    }

    fn period_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.5 * (1.0 + eccentricity) / (1.0 - eccentricity) + 0.5)
    }

    /// Eccentricity itself: de = (1+e) dmu/mu at a fixed perigee state.
    fn ecc_abs_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity)
    }

    /// Terra's GM (m^3/s^2), only for constructing test states.
    const MU_M3_S2: f64 = 3.986004418e14;

    fn satkit_vec(v: DVec3) -> satkit::Vector3 {
        satkit::Vector3::new([[v.x], [v.y], [v.z]])
    }

    /// Perigee states across eccentricities: v_perigee for eccentricity e
    /// at radius r is sqrt(mu (1 + e) / r).
    fn perigee_state(radius_m: f64, eccentricity: f64) -> (DVec3, DVec3) {
        let speed = (MU_M3_S2 * (1.0 + eccentricity) / radius_m).sqrt();
        (DVec3::new(radius_m, 0.0, 0.0), DVec3::new(0.0, speed, 0.0))
    }

    #[test]
    fn elements_match_satkit() {
        crate::data::seed_satkit();
        for eccentricity in [0.0, 0.3, 0.7, 0.99] {
            let (pos, vel) = perigee_state(6_778_000.0, eccentricity);
            let ours = Kepler::from_pv(crate::data::astro(), pos, vel)
                .unwrap_or_else(|e| panic!("crate elements at e = {eccentricity}: {e}"));
            let theirs = satkit::Kepler::from_pv(satkit_vec(pos), satkit_vec(vel))
                .unwrap_or_else(|e| panic!("satkit elements at e = {eccentricity}: {e}"));
            let label = format!("e = {eccentricity}");
            assert!(
                (ours.semi_major_axis_m - theirs.a).abs() / theirs.a < sma_rel_tol(eccentricity),
                "{label}: a {} vs {}",
                ours.semi_major_axis_m,
                theirs.a
            );
            assert!(
                (ours.eccentricity - theirs.eccen).abs() < ecc_abs_tol(eccentricity),
                "{label}: e {} vs {}",
                ours.eccentricity,
                theirs.eccen
            );
            let want_period = theirs.period();
            assert!(
                (ours.period_s - want_period).abs() / want_period < period_rel_tol(eccentricity),
                "{label}: period {} vs {want_period}",
                ours.period_s
            );
        }
    }

    /// Escape states must err on BOTH sides (the crate imposes its own
    /// gate; satkit errs inherently).
    #[test]
    fn escape_state_errs_on_both_sides() {
        crate::data::seed_satkit();
        let (pos, vel) = perigee_state(6_778_000.0, 1.2);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_err());
        assert!(satkit::Kepler::from_pv(satkit_vec(pos), satkit_vec(vel)).is_err());
    }
}

/// The astrodyn (JEOD-port) reference side.
mod astrodyn {
    use astrodyn_math::OrbitalElements;
    use astrodyn_quantities::frame::{Earth, PlanetInertial};
    use astrodyn_quantities::prelude::*;
    use engine_astrodynamics::kepler::Kepler;
    use glam::DVec3;

    use crate::support::astrodyn_vec;

    /// Same mu-provenance reasoning as the satkit kepler module: astrodyn
    /// is handed the satkit-class mu below, so the crate's pca-derived mu
    /// differs by the same ~1.6e-8 relative gap, amplified into the
    /// elements at a fixed perigee state (see that module's derivation).
    const MU_PROVENANCE_REL: f64 = 1.61e-8;

    fn sma_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity) / (1.0 - eccentricity)
    }

    fn period_rel_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.5 * (1.0 + eccentricity) / (1.0 - eccentricity) + 0.5)
    }

    fn ecc_abs_tol(eccentricity: f64) -> f64 {
        3.0 * MU_PROVENANCE_REL * (1.0 + eccentricity)
    }

    /// Terra's GM (m^3/s^2), for test states AND as astrodyn's handed mu.
    const MU_M3_S2: f64 = 3.986004418e14;

    fn perigee_state(radius_m: f64, eccentricity: f64) -> (DVec3, DVec3) {
        let speed = (MU_M3_S2 * (1.0 + eccentricity) / radius_m).sqrt();
        (DVec3::new(radius_m, 0.0, 0.0), DVec3::new(0.0, speed, 0.0))
    }

    fn astrodyn_elements(pos: DVec3, vel: DVec3) -> OrbitalElements<Earth> {
        OrbitalElements::from_cartesian_typed(
            MU_M3_S2.m3_per_s2_for::<Earth>(),
            astrodyn_vec(pos).m_at::<PlanetInertial<Earth>>(),
            astrodyn_vec(vel).m_per_s_at::<PlanetInertial<Earth>>(),
        )
        .expect("astrodyn elements")
    }

    /// e stops at 0.95 (not the satkit module's 0.99): JEOD declares
    /// |e - 1| <= 0.01 parabolic and nulls the semi-major axis — see the
    /// contract-difference test below.
    #[test]
    fn elements_match_astrodyn() {
        for eccentricity in [0.0, 0.3, 0.7, 0.95] {
            let (pos, vel) = perigee_state(6_778_000.0, eccentricity);
            let ours = Kepler::from_pv(crate::data::astro(), pos, vel)
                .unwrap_or_else(|e| panic!("crate elements at e = {eccentricity}: {e}"));
            let theirs = astrodyn_elements(pos, vel);
            let label = format!("e = {eccentricity}");
            assert!(
                (ours.semi_major_axis_m - theirs.semi_major_axis).abs() / theirs.semi_major_axis
                    < sma_rel_tol(eccentricity),
                "{label}: a {} vs {}",
                ours.semi_major_axis_m,
                theirs.semi_major_axis
            );
            assert!(
                (ours.eccentricity - theirs.e_mag).abs() < ecc_abs_tol(eccentricity),
                "{label}: e {} vs {}",
                ours.eccentricity,
                theirs.e_mag
            );
            let want_period = std::f64::consts::TAU / theirs.mean_motion;
            assert!(
                (ours.period_s - want_period).abs() / want_period < period_rel_tol(eccentricity),
                "{label}: period {} vs {want_period}",
                ours.period_s
            );
        }
    }

    /// Contract differences, pinned: the crate refuses escape states (its
    /// e >= 1 gate) while JEOD's element set is defined for hyperbolic
    /// orbits (negative semi-major axis) - astrodyn must succeed and
    /// report the same eccentricity regime, not err. And inside JEOD's
    /// parabolic band (|e - 1| <= its ORBIT_SWITCH_TOL of 1e-2) astrodyn
    /// nulls the semi-major axis to 0.0 where the crate still returns the
    /// finite ellipse - which is why the element comparison above stops
    /// at e = 0.95.
    #[test]
    fn escape_and_parabolic_band_contract_differences() {
        let (pos, vel) = perigee_state(6_778_000.0, 1.2);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_err());
        let theirs = astrodyn_elements(pos, vel);
        assert!(
            theirs.e_mag > 1.0 && theirs.semi_major_axis < 0.0,
            "astrodyn hyperbolic elements: e {} a {}",
            theirs.e_mag,
            theirs.semi_major_axis
        );

        let (pos, vel) = perigee_state(6_778_000.0, 0.99);
        assert!(Kepler::from_pv(crate::data::astro(), pos, vel).is_ok());
        let theirs = astrodyn_elements(pos, vel);
        assert!(
            theirs.semi_major_axis == 0.0 && theirs.semiparam > 0.0,
            "astrodyn parabolic-band elements: a {} p {}",
            theirs.semi_major_axis,
            theirs.semiparam
        );
    }
}
