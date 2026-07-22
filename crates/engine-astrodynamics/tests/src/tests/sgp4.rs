//! Crate-vs-reference SGP4 comparisons — satkit only: astrodyn has no
//! SGP4.

/// The satkit reference side.
mod satkit {
    use engine_astrodynamics::{Duration, Epoch, sgp4::sgp4, tle::Tle};

    use crate::support::satkit_instant;

    /// Harness-owned ISS fixture (real checksums - the `sgp4` crate
    /// validates them; satkit ignores them).
    const ISS_TLE: [&str; 3] = [
        "ISS (ZARYA)",
        "1 25544U 98067A   24001.50000000  .00016717  00000-0  10270-3 0  9009",
        "2 25544  51.6432 351.4697 0007417 130.5364 329.6482 15.48915330299350",
    ];

    /// Both sides are validated against the Vallado reference C++, so a
    /// miss here means a geopotential-constants (WGS72/84) or time-bridge
    /// bug, not physics.
    const POSITION_TOL_M: f64 = 10.0;
    const VELOCITY_TOL_M_S: f64 = 0.01;

    #[test]
    fn matches_satkit_over_a_two_week_window() {
        crate::data::seed_satkit();
        let ours_tle = Tle::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");
        let mut theirs_tle =
            satkit::tle::TLE::load_3line(ISS_TLE[0], ISS_TLE[1], ISS_TLE[2]).expect("valid TLE");

        // +-1 week around the element-set epoch at 3-hour steps.
        let t0 = ours_tle.epoch();
        let epochs: Vec<Epoch> = (-56..=56)
            .map(|i| t0 + Duration::from_seconds(3.0 * 3600.0 * f64::from(i)))
            .collect();
        let ours = sgp4(&ours_tle, &epochs).expect("crate sgp4");

        let instants: Vec<satkit::Instant> =
            epochs.iter().map(|&epoch| satkit_instant(epoch)).collect();
        let theirs = satkit::sgp4::sgp4(&mut theirs_tle, &instants).expect("satkit sgp4");

        for (i, state) in ours.iter().enumerate() {
            let want_pos =
                glam::DVec3::new(theirs.pos[(0, i)], theirs.pos[(1, i)], theirs.pos[(2, i)]);
            let want_vel =
                glam::DVec3::new(theirs.vel[(0, i)], theirs.vel[(1, i)], theirs.vel[(2, i)]);
            let pos_diff = (state.pos_teme_m - want_pos).length();
            let vel_diff = (state.vel_teme_m_s - want_vel).length();
            assert!(
                pos_diff <= POSITION_TOL_M,
                "sample {i}: position differs by {pos_diff:.3} m"
            );
            assert!(
                vel_diff <= VELOCITY_TOL_M_S,
                "sample {i}: velocity differs by {vel_diff:.5} m/s"
            );
        }
    }
}
