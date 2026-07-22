//! Rotations between the inertial (GCRF), SGP4-native (TEME), and
//! Earth-fixed (ITRF) frames as glam quaternions, over the embedded
//! high-precision Earth PCK (ITRF93: nutation, polar motion, and UT1 baked
//! in by NAIF, pre-resolved by segments.rs) and the IAU-76/FK5 TEME chain
//! evaluated directly through sofars - the same SOFA routines anise's
//! dynamic-frame dispatch runs, called once per query instead of six times
//! (anise finite-differences a rotation derivative this positions-only
//! quaternion surface discards, and its equation-of-the-equinoxes call
//! re-runs the nut80 series its nutation matrix already evaluated).

use anise::math::Matrix3;
use anise::math::rotation::{DCM, r3};
use glam::{DMat3, DQuat, DVec3};
use hifitime::{Epoch, Unit};
use sofars::consts::{D2PI, DAS2R, DJ00, DJC};
use sofars::pnp::{numat, nut80, obl80, pmat76};
use sofars::vm::anpm;

use crate::data::AstroData;

/// GCRF -> ITRF, apply as `q * v`. Positions only: the Earth-fixed frame
/// rotates, so a velocity additionally needs the omega-cross term.
pub fn qgcrf2itrf(data: &AstroData, epoch: Epoch) -> DQuat {
    dquat_mat(&itrf_dcm(data, epoch).rot_mat)
}

/// ITRF -> GCRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qitrf2gcrf(data: &AstroData, epoch: Epoch) -> DQuat {
    dquat_mat(&itrf_dcm(data, epoch).rot_mat.transpose())
}

/// TEME -> GCRF, apply as `q * v`. Both frames are quasi-inertial, so
/// rotating a velocity by the same quaternion is correct. TEME is the
/// LEGACY model (IAU-76/FK5 precession + 1980 nutation, the SGP4-matching
/// convention), not the IAU2006-class variant - identical numerics to
/// anise's `EARTH_TEME_LEGACY_FRAME` (pinned by the test below).
pub fn qteme2gcrf(_data: &AstroData, epoch: Epoch) -> DQuat {
    dquat_mat(&j2000_to_teme(epoch).transpose())
}

/// TEME -> ITRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qteme2itrf(data: &AstroData, epoch: Epoch) -> DQuat {
    dquat_mat(&(itrf_dcm(data, epoch).rot_mat * j2000_to_teme(epoch).transpose()))
}

/// Panics outside the embedded Earth PCK's span (1962..2125): the frame
/// surface stays infallible like the satkit era, and every scene is
/// EOP-gated well inside that window.
fn itrf_dcm(data: &AstroData, epoch: Epoch) -> DCM {
    data.earth_rotation
        .dcm_j2000_to_itrf93(epoch)
        .unwrap_or_else(|error| {
            panic!("rotation GCRF -> ITRF at {epoch}: {error} (Earth PCK spans 1962..2125)")
        })
}

/// The J2000 -> TEME matrix of anise's legacy model, evaluated in one
/// pass: `r3(eqeq) * nutation * precession` with the IAU-1980 nutation
/// series run ONCE and shared between the nutation matrix and the 1994
/// equation of the equinoxes (sofars `eqeq94` recomputes `nut80` from the
/// same date, so sharing `dpsi` is value-identical).
fn j2000_to_teme(epoch: Epoch) -> Matrix3 {
    let (tt1, tt2) = sofa_tt_jd_parts(epoch);
    let (dpsi, deps) = nut80(tt1, tt2);
    let mean_obliquity = obl80(tt1, tt2);
    let nutation = mat3(&numat(mean_obliquity, dpsi, deps));
    let precession = mat3(&pmat76(tt1, tt2));

    // sofars `eqeq94` body with the shared nutation-in-longitude: the mean
    // ascending lunar node's longitude, then IAU (1994) Resolution C7.
    let t = ((tt1 - DJ00) + tt2) / DJC;
    let node_rad = anpm(
        (450160.280 + (-482890.539 + (7.455 + 0.008 * t) * t) * t) * DAS2R
            + ((-5.0 * t) % 1.0) * D2PI,
    );
    let eqeq = dpsi * mean_obliquity.cos()
        + DAS2R * (0.00264 * node_rad.sin() + 0.000063 * (node_rad + node_rad).sin());

    r3(eqeq) * (nutation * precession)
}

/// SOFA's two-part TT Julian date, split exactly as anise does for its
/// dynamic frames (whole days + day fraction) so the series see the same
/// arguments.
fn sofa_tt_jd_parts(epoch: Epoch) -> (f64, f64) {
    let jde_tt = epoch.to_jde_tt_duration();
    let tt1 = jde_tt.to_unit(Unit::Day).trunc();
    let tt2 = (jde_tt - Unit::Day * tt1).to_unit(Unit::Day);
    (tt1, tt2)
}

/// sofars returns row-major `[[f64; 3]; 3]`; nalgebra's `Matrix3::new`
/// fills row by row (the same conversion anise's dynamic frames use).
fn mat3(m: &[[f64; 3]; 3]) -> Matrix3 {
    Matrix3::new(
        m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
    )
}

/// anise's DCM applies as `v_to = rot_mat * v_from`; rebuild it column by
/// column for glam (`from_cols` takes COLUMNS - feeding rows would produce
/// the transpose, i.e. the inverse rotation; the chirality test below and
/// the harness angle comparisons both catch that mistake).
pub(crate) fn dquat(dcm: &DCM) -> DQuat {
    dquat_mat(&dcm.rot_mat)
}

fn dquat_mat(m: &Matrix3) -> DQuat {
    DQuat::from_mat3(&DMat3::from_cols(
        DVec3::new(m[(0, 0)], m[(1, 0)], m[(2, 0)]),
        DVec3::new(m[(0, 1)], m[(1, 1)], m[(2, 1)]),
        DVec3::new(m[(0, 2)], m[(1, 2)], m[(2, 2)]),
    ))
    .normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::test_data;

    fn epoch() -> Epoch {
        Epoch::from_gregorian_utc(2024, 6, 15, 0, 0, 0, 0)
    }

    /// GCRF -> ITRF -> GCRF must return the original vector, and the
    /// rotation must preserve length.
    #[test]
    fn gcrf_itrf_round_trip() {
        let data = test_data();
        let time = epoch();
        let v = DVec3::new(1.0, 2.0, 3.0).normalize();
        let there = qgcrf2itrf(data, time) * v;
        assert!((there.length() - 1.0).abs() < 1e-12);
        assert!((qitrf2gcrf(data, time) * there - v).length() < 1e-9);
    }

    /// The direct sofars TEME chain must reproduce anise's legacy dynamic
    /// frame - same SOFA routines, one evaluation instead of six. Any real
    /// divergence means the shared-`dpsi` equation of the equinoxes or the
    /// matrix assembly drifted from `EARTH_TEME_LEGACY_FRAME`.
    #[test]
    fn teme_matches_anise_dynamic_frame() {
        use anise::constants::frames::{EARTH_J2000, EARTH_TEME_LEGACY_FRAME};
        let data = test_data();
        for year in [1965, 1994, 2024, 2100] {
            let time = Epoch::from_gregorian_utc(year, 6, 15, 3, 30, 0, 0);
            let reference = dquat(
                &data
                    .almanac
                    .rotate(EARTH_TEME_LEGACY_FRAME, EARTH_J2000, time)
                    .expect("anise TEME rotation"),
            );
            let angle = qteme2gcrf(data, time).angle_between(reference);
            assert!(angle < 1e-12, "TEME diverged by {angle} rad at {time}");
        }
    }

    /// The two TEME routes into ITRF must agree: direct, and via GCRF.
    #[test]
    fn teme_routes_agree() {
        let data = test_data();
        let time = epoch();
        let v = DVec3::new(0.3, -0.8, 0.52).normalize();
        let via_gcrf = qgcrf2itrf(data, time) * (qteme2gcrf(data, time) * v);
        let direct = qteme2itrf(data, time) * v;
        // qteme2gcrf is the ~arcsec-class transform; allow that error scale.
        assert!(
            (via_gcrf - direct).length() < 1e-4,
            "routes differ by {}",
            (via_gcrf - direct).length()
        );
    }

    /// The GCRF pole maps near the ITRF pole: precession since J2000 plus
    /// polar motion is well under 0.2 deg at this epoch.
    #[test]
    fn pole_stays_near_pole() {
        let pole = qgcrf2itrf(test_data(), epoch()) * DVec3::Z;
        assert!(
            pole.z > (0.2_f64.to_radians()).cos(),
            "pole tilted to {pole}"
        );
    }

    /// Terra rotates: six hours apart, the same GCRF direction lands ~90
    /// deg apart in the Earth-fixed frame.
    #[test]
    fn earth_rotation_advances() {
        let data = test_data();
        let t0 = epoch();
        let t1 = t0 + crate::Duration::from_seconds(6.0 * 3600.0);
        let a = qgcrf2itrf(data, t0) * DVec3::X;
        let b = qgcrf2itrf(data, t1) * DVec3::X;
        let angle = a.angle_between(b).to_degrees();
        assert!(
            (85.0..95.0).contains(&angle),
            "rotated {angle:.2} deg in 6 h"
        );
    }

    /// Terra rotates EASTWARD, so a fixed GCRF direction sweeps westward
    /// (retrograde) in the Earth-fixed frame: `a x b` must point south.
    /// The round-trip and angle tests above are symmetric in q vs q^-1 -
    /// only this catches a transposed DCM (the inverse rotation).
    #[test]
    fn rotation_chirality_is_eastward() {
        let data = test_data();
        let t0 = epoch();
        let t1 = t0 + crate::Duration::from_seconds(3600.0);
        let a = qgcrf2itrf(data, t0) * DVec3::X;
        let b = qgcrf2itrf(data, t1) * DVec3::X;
        assert!(
            a.cross(b).z < 0.0,
            "fixed GCRF direction swept eastward in ITRF - DCM transposed?"
        );
    }
}
