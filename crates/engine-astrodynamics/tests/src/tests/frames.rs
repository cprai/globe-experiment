//! Crate-vs-reference frame-rotation comparisons.

/// The satkit reference side.
mod satkit {
    use engine_astrodynamics::{Epoch, frames};
    use glam::DQuat;

    use crate::support::satkit_instant;

    /// The GCRF<->ITRF bound: the crate's ITRF93 BPC vs satkit's
    /// IERS-2010 + CelesTrak EOP are different Earth-orientation
    /// realizations; the kernel's own historical accuracy claim is
    /// < 3 urad, and in practice the realizations sit within ~0.2 arcsec
    /// of each other post-1972 (looser before, when EOP data thins out).
    /// Measured worst 2026-07-21: 0.015 arcsec (1980 epoch).
    const GCRF_ITRF_ARCSEC: f64 = 0.05;
    /// Width of the pinned pre-1972 divergence window (see the test doc).
    const GCRF_ITRF_PRE_1972_WINDOW_ARCSEC: f64 = 2.0;
    /// TEME: both sides are equinox-based IAU-76/FK5-class chains.
    /// Measured worst 2026-07-21: 0.22 arcsec (1980 epoch).
    const TEME_ARCSEC: f64 = 0.5;

    fn epochs() -> [Epoch; 4] {
        [
            (1980, 6, 1, 0, 0, 0),
            (2000, 1, 1, 12, 0, 0),
            (2012, 3, 15, 6, 30, 0),
            (2024, 1, 15, 12, 30, 0),
        ]
        .map(|(year, month, day, hour, minute, second)| {
            Epoch::from_gregorian_utc(year, month, day, hour, minute, second, 0)
        })
    }

    fn quat(q: satkit::Quaternion) -> DQuat {
        DQuat::from_xyzw(q.x, q.y, q.z, q.w)
    }

    /// Angle of the relative rotation between two unit quaternions, arcsec.
    fn relative_arcsec(a: DQuat, b: DQuat) -> f64 {
        let rel = a * b.inverse();
        2.0 * rel.w.abs().clamp(0.0, 1.0).acos().to_degrees() * 3600.0
    }

    #[test]
    fn gcrf_itrf_matches_satkit() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let angle = relative_arcsec(
                frames::qgcrf2itrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qgcrf2itrf(&satkit_instant(epoch))),
            );
            assert!(
                angle <= GCRF_ITRF_ARCSEC,
                "qgcrf2itrf epoch {index}: {angle:.4} arcsec"
            );
        }
    }

    /// Before 1972, UTC ran on the rubber-second regime and the two stacks
    /// model it differently: satkit treats pre-1972 TAI-UTC as zero, while
    /// the NAIF BPC on the crate's side carries the true historical time -
    /// on 1965-06-01, TAI-UTC = 3.640130 s + 151 days x 1.296 ms/day
    /// = 3.836 s, which times Terra's 15.041 arcsec/s rotation rate
    /// predicts a 57.70 arcsec divergence. The comparison therefore PINS
    /// that predicted window rather than asserting closeness: the crate
    /// regressing (or satkit gaining a pre-1972 UTC model) both surface as
    /// a moved angle. Post-1972 closeness is the real correctness gate
    /// above.
    #[test]
    fn gcrf_itrf_pre_1972_diverges_by_the_rubber_second_offset() {
        crate::data::seed_satkit();
        let epoch = Epoch::from_gregorian_utc(1965, 6, 1, 0, 0, 0, 0);
        let angle = relative_arcsec(
            frames::qgcrf2itrf(crate::data::astro(), epoch),
            quat(satkit::frametransform::qgcrf2itrf(&satkit_instant(epoch))),
        );
        let predicted = 15.041 * (3.640_130 + 151.0 * 0.001_296);
        assert!(
            (angle - predicted).abs() < GCRF_ITRF_PRE_1972_WINDOW_ARCSEC,
            "qgcrf2itrf 1965: {angle:.4} arcsec, predicted {predicted:.2}"
        );
    }

    #[test]
    fn teme_matches_satkit() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let instant = satkit_instant(epoch);
            let to_gcrf = relative_arcsec(
                frames::qteme2gcrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qteme2gcrf(&instant)),
            );
            assert!(
                to_gcrf <= TEME_ARCSEC,
                "qteme2gcrf epoch {index}: {to_gcrf:.4} arcsec"
            );
            let to_itrf = relative_arcsec(
                frames::qteme2itrf(crate::data::astro(), epoch),
                quat(satkit::frametransform::qteme2itrf(&instant)),
            );
            assert!(
                to_itrf <= TEME_ARCSEC,
                "qteme2itrf epoch {index}: {to_itrf:.4} arcsec"
            );
        }
    }
}

/// The astrodyn (JEOD-port) reference side.
mod astrodyn {
    use engine_astrodynamics::{Epoch, frames};
    use glam::DQuat;

    use crate::support::{astrodyn_rnp_inputs, quat_from_astrodyn_mat};

    /// The crate's ITRF93 BPC vs astrodyn's JEOD RNP (IAU-76/FK5
    /// precession + 1980 nutation + Aoki GMST + polar motion from the
    /// satkit EOP table). Different Earth-orientation realizations AND a
    /// different UT1 source, plus the ~23 mas J2000<->GCRF frame bias the
    /// JEOD chain does not model. Measured worst 2026-07-22: 0.048 arcsec
    /// (2024 epoch); ~4x headroom.
    const GCRF_ITRF_ARCSEC: f64 = 0.2;

    /// Post-1972 epochs only: the RNP inputs come from the satkit EOP
    /// table, which zeroes the pre-1972 rubber-second regime (see the
    /// pinned 1965 divergence test in the satkit frames module).
    fn epochs() -> [Epoch; 4] {
        [
            (1980, 6, 1, 0, 0, 0),
            (2000, 1, 1, 12, 0, 0),
            (2012, 3, 15, 6, 30, 0),
            (2024, 1, 15, 12, 30, 0),
        ]
        .map(|(year, month, day, hour, minute, second)| {
            Epoch::from_gregorian_utc(year, month, day, hour, minute, second, 0)
        })
    }

    fn relative_arcsec(a: DQuat, b: DQuat) -> f64 {
        let rel = a * b.inverse();
        2.0 * rel.w.abs().clamp(0.0, 1.0).acos().to_degrees() * 3600.0
    }

    #[test]
    fn gcrf_itrf_matches_astrodyn_rnp() {
        crate::data::seed_satkit();
        for (index, &epoch) in epochs().iter().enumerate() {
            let (gmst_seconds, tt_centuries, polar) = astrodyn_rnp_inputs(epoch);
            let reference = quat_from_astrodyn_mat(
                astrodyn_frames::rotation_j2000::compute_t_parent_this_with_polar(
                    gmst_seconds,
                    tt_centuries,
                    Some(polar),
                ),
            );
            let angle = relative_arcsec(frames::qgcrf2itrf(crate::data::astro(), epoch), reference);
            assert!(
                angle <= GCRF_ITRF_ARCSEC,
                "qgcrf2itrf vs astrodyn RNP epoch {index}: {angle:.4} arcsec"
            );
        }
    }
}
