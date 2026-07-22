//! Rotations between the inertial (GCRF), SGP4-native (TEME), and
//! Earth-fixed (ITRF) frames as glam quaternions, over the embedded
//! high-precision Earth PCK (ITRF93: nutation, polar motion, and UT1 baked
//! in by NAIF) and anise's analytic TEME model.

use anise::constants::frames::{EARTH_ITRF93, EARTH_J2000, EARTH_TEME_LEGACY_FRAME};
use anise::frames::Frame;
use anise::math::rotation::DCM;
use glam::{DMat3, DQuat, DVec3};
use hifitime::Epoch;

use crate::data::AstroData;

/// GCRF -> ITRF, apply as `q * v`. Positions only: the Earth-fixed frame
/// rotates, so a velocity additionally needs the omega-cross term.
pub fn qgcrf2itrf(data: &AstroData, epoch: Epoch) -> DQuat {
    rotation(data, EARTH_J2000, EARTH_ITRF93, epoch)
}

/// ITRF -> GCRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qitrf2gcrf(data: &AstroData, epoch: Epoch) -> DQuat {
    rotation(data, EARTH_ITRF93, EARTH_J2000, epoch)
}

/// TEME -> GCRF, apply as `q * v`. Both frames are quasi-inertial, so
/// rotating a velocity by the same quaternion is correct. TEME here is
/// anise's LEGACY model (IAU-76/FK5 precession + 1980 nutation, the
/// SGP4-matching convention), not the IAU2006-class variant.
pub fn qteme2gcrf(data: &AstroData, epoch: Epoch) -> DQuat {
    rotation(data, EARTH_TEME_LEGACY_FRAME, EARTH_J2000, epoch)
}

/// TEME -> ITRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qteme2itrf(data: &AstroData, epoch: Epoch) -> DQuat {
    rotation(data, EARTH_TEME_LEGACY_FRAME, EARTH_ITRF93, epoch)
}

/// Panics outside the embedded Earth PCK's span (1962..2125): the frame
/// surface stays infallible like the satkit era, and every scene is
/// EOP-gated well inside that window.
fn rotation(data: &AstroData, from: Frame, to: Frame, epoch: Epoch) -> DQuat {
    let dcm = data
        .almanac
        .rotate(from, to, epoch)
        .unwrap_or_else(|error| {
            panic!("rotation {from} -> {to} at {epoch}: {error} (Earth PCK spans 1962..2125)")
        });
    dquat(&dcm)
}

/// anise's DCM applies as `v_to = rot_mat * v_from`; rebuild it column by
/// column for glam (`from_cols` takes COLUMNS - feeding rows would produce
/// the transpose, i.e. the inverse rotation; the chirality test below and
/// the harness angle comparisons both catch that mistake).
pub(crate) fn dquat(dcm: &DCM) -> DQuat {
    let m = &dcm.rot_mat;
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
