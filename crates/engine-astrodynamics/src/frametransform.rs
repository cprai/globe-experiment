//! Rotations between the inertial (GCRF), SGP4-native (TEME), and
//! Earth-fixed (ITRF) frames as glam quaternions, delegating to satkit's
//! full IERS-2010 transforms (real EOP - requires [`crate::init`]).

use glam::DQuat;
use satkit::Instant;
use satkit::frametransform as sk;

/// GCRF -> ITRF, apply as `q * v`. Positions only: the Earth-fixed frame
/// rotates, so a velocity additionally needs the omega-cross term.
pub fn qgcrf2itrf(time: &Instant) -> DQuat {
    quat(sk::qgcrf2itrf(time))
}

/// ITRF -> GCRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qitrf2gcrf(time: &Instant) -> DQuat {
    quat(sk::qitrf2gcrf(time))
}

/// TEME -> GCRF, apply as `q * v`. Both frames are quasi-inertial, so
/// rotating a velocity by the same quaternion is correct.
pub fn qteme2gcrf(time: &Instant) -> DQuat {
    quat(sk::qteme2gcrf(time))
}

/// TEME -> ITRF, apply as `q * v`. Positions only (see [`qgcrf2itrf`]).
pub fn qteme2itrf(time: &Instant) -> DQuat {
    quat(sk::qteme2itrf(time))
}

fn quat(q: satkit::Quaternion) -> DQuat {
    DQuat::from_xyzw(q.x, q.y, q.z, q.w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init;
    use glam::DVec3;

    fn epoch() -> Instant {
        Instant::from_datetime(2024, 6, 15, 0, 0, 0.0).expect("valid test epoch")
    }

    /// GCRF -> ITRF -> GCRF must return the original vector, and the
    /// rotation must preserve length.
    #[test]
    fn gcrf_itrf_round_trip() {
        init();
        let time = epoch();
        let v = DVec3::new(1.0, 2.0, 3.0).normalize();
        let there = qgcrf2itrf(&time) * v;
        assert!((there.length() - 1.0).abs() < 1e-12);
        assert!((qitrf2gcrf(&time) * there - v).length() < 1e-9);
    }

    /// The two TEME routes into ITRF must agree: direct, and via GCRF.
    #[test]
    fn teme_routes_agree() {
        init();
        let time = epoch();
        let v = DVec3::new(0.3, -0.8, 0.52).normalize();
        let via_gcrf = qgcrf2itrf(&time) * (qteme2gcrf(&time) * v);
        let direct = qteme2itrf(&time) * v;
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
        init();
        let pole = qgcrf2itrf(&epoch()) * DVec3::Z;
        assert!(
            pole.z > (0.2_f64.to_radians()).cos(),
            "pole tilted to {pole}"
        );
    }

    /// Terra rotates: six hours apart, the same GCRF direction lands ~90
    /// deg apart in the Earth-fixed frame.
    #[test]
    fn earth_rotation_advances() {
        init();
        let t0 = epoch();
        let t1 = t0 + satkit::Duration::from_seconds(6.0 * 3600.0);
        let a = qgcrf2itrf(&t0) * DVec3::X;
        let b = qgcrf2itrf(&t1) * DVec3::X;
        let angle = a.angle_between(b).to_degrees();
        assert!(
            (85.0..95.0).contains(&angle),
            "rotated {angle:.2} deg in 6 h"
        );
    }
}
