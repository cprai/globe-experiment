//! Conical shadow with penumbra (spec §4.5): the illumination fraction
//! `nu` in [0, 1] from the apparent angular radii of the Sun and the
//! occulting body as seen from the spacecraft. The Sun's ~0.267 deg
//! apparent radius makes the penumbra a smooth ramp - a cylindrical model
//! would put a C0 discontinuity through DOP853's error estimator.
//!
//! Occulters are treated as spheres; the ellipsoidal-Earth refinement
//! (eclipse boundary displaced by tens of km, i.e. seconds of transit
//! timing) sits below the accuracy target for the tracked-arc regimes and
//! is deliberately deferred, as are atmospheric refraction and ozone
//! absorption (spec lists both as later enhancements).

use glam::DVec3;

/// Illumination fraction seen by the spacecraft against ONE occulter:
/// 1 in full sun, 0 in umbra, the circle-circle lens ramp in penumbra,
/// and the annular residual in the antumbra. Multiple occulters combine
/// as `nu = prod nu_i` (exact while they don't overlap on the solar disk).
pub(crate) fn illumination(
    to_sun_m: DVec3,
    to_occulter_m: DVec3,
    sun_radius_m: f64,
    occulter_radius_m: f64,
) -> f64 {
    let d_sun = to_sun_m.length();
    let d_occ = to_occulter_m.length();
    // No occultation from a body farther than the Sun itself.
    if d_occ >= d_sun {
        return 1.0;
    }
    // At or below the occulter's surface the disk fills the sky.
    if d_occ <= occulter_radius_m {
        return 0.0;
    }

    let sun_apparent = (sun_radius_m / d_sun).min(1.0).asin();
    let occ_apparent = (occulter_radius_m / d_occ).min(1.0).asin();
    let separation = to_sun_m.angle_between(to_occulter_m);

    if separation >= sun_apparent + occ_apparent {
        return 1.0; // fully clear
    }
    if separation <= occ_apparent - sun_apparent {
        return 0.0; // umbra: the occulter covers the whole solar disk
    }
    if separation <= sun_apparent - occ_apparent {
        // Antumbra: the occulter sits entirely on the solar disk.
        return 1.0 - (occ_apparent / sun_apparent).powi(2);
    }
    // Penumbra: partial overlap, planar small-angle disk geometry.
    let blocked = lens_area(sun_apparent, occ_apparent, separation);
    (1.0 - blocked / (std::f64::consts::PI * sun_apparent * sun_apparent)).clamp(0.0, 1.0)
}

/// Intersection ("lens") area of two disks with radii `r1`, `r2` and
/// center distance `d`, in the partial-overlap regime.
fn lens_area(r1: f64, r2: f64, d: f64) -> f64 {
    let alpha = ((d * d + r1 * r1 - r2 * r2) / (2.0 * d * r1))
        .clamp(-1.0, 1.0)
        .acos();
    let beta = ((d * d + r2 * r2 - r1 * r1) / (2.0 * d * r2))
        .clamp(-1.0, 1.0)
        .acos();
    let kite = ((-d + r1 + r2) * (d + r1 - r2) * (d - r1 + r2) * (d + r1 + r2)).max(0.0);
    r1 * r1 * alpha + r2 * r2 * beta - 0.5 * kite.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUN_RADIUS_M: f64 = 6.957e8;
    const EARTH_RADIUS_M: f64 = 6.378e6;
    const AU_M: f64 = 1.495_978_707e11;

    /// nu against a LEO-scale Earth as the occulter's apparent position
    /// sweeps from dead-center on the Sun (umbra - the geometry of an
    /// anti-sunward spacecraft, whose to-occulter vector points TOWARD the
    /// Sun) out to 90 deg away (clear): exactly 0 deep inside, exactly 1
    /// well outside, monotone and continuous through the penumbra
    /// (spec §7.8).
    #[test]
    fn continuous_monotone_ramp_across_the_penumbra() {
        let to_sun = DVec3::X * AU_M;
        let d_occ = 7.0e6;
        let steps = 200_000;
        let mut previous = 0.0;
        let mut seen_partial = false;
        for i in 0..=steps {
            let angle = std::f64::consts::FRAC_PI_2 * f64::from(i) / f64::from(steps);
            let to_occ = DVec3::new(angle.cos(), angle.sin(), 0.0) * d_occ;
            let nu = illumination(to_sun, to_occ, SUN_RADIUS_M, EARTH_RADIUS_M);
            assert!((0.0..=1.0).contains(&nu), "nu out of range: {nu}");
            assert!(
                nu >= previous - 1e-9,
                "nu not monotone at angle {angle}: {nu} < {previous}"
            );
            assert!(
                (nu - previous).abs() < 2e-3,
                "nu jumped at angle {angle}: {previous} -> {nu}"
            );
            if nu > 0.0 && nu < 1.0 {
                seen_partial = true;
            }
            previous = nu;
        }
        assert!(seen_partial, "sweep never crossed the penumbra");
        assert_eq!(
            illumination(to_sun, DVec3::X * d_occ, SUN_RADIUS_M, EARTH_RADIUS_M),
            0.0,
            "occulter centered on the solar disk is umbra"
        );
        assert_eq!(
            illumination(to_sun, DVec3::Y * d_occ, SUN_RADIUS_M, EARTH_RADIUS_M),
            1.0,
            "occulter 90 deg off the Sun is clear"
        );
    }

    /// Beyond the umbra vertex (~1.4e9 m for Earth) a perfectly aligned
    /// spacecraft sees an annular eclipse: 0 < nu < 1, approaching 1 with
    /// distance.
    #[test]
    fn antumbra_beyond_the_umbra_vertex() {
        let to_sun = DVec3::X * AU_M;
        let near = illumination(
            to_sun,
            DVec3::X * 2.0e9, // toward the Sun, past the ~1.4e9 m vertex
            SUN_RADIUS_M,
            EARTH_RADIUS_M,
        );
        assert!(
            (0.0..1.0).contains(&near) && near > 0.0,
            "annular nu = {near}"
        );
        let far = illumination(to_sun, DVec3::X * 2.0e10, SUN_RADIUS_M, EARTH_RADIUS_M);
        assert!(far > near && far < 1.0, "annular nu must recover: {far}");
    }

    /// A body farther than the Sun never occults, wherever it appears.
    #[test]
    fn occulter_behind_the_sun_is_ignored() {
        let nu = illumination(
            DVec3::X * AU_M,
            DVec3::X * (2.0 * AU_M),
            SUN_RADIUS_M,
            6.0e8,
        );
        assert_eq!(nu, 1.0);
    }
}
