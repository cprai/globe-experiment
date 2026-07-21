//! EGM2008 spherical-harmonic gravity (spec §4.1): the packed-coefficient
//! loader, a fully normalized Pines evaluation (regular at the poles), and
//! the frequency-independent degree-2 solid-tide correction.
//!
//! Everything here evaluates with the MODEL'S OWN defining constants (GM,
//! a from the packed header) - never the canonical-unit mu; the two differ
//! legitimately in the low digits (spec §1/§4.1).
//!
//! # Pines formulation (derived, fully normalized)
//!
//! With `s = x/r`, `t = y/r`, `u = z/r`, `R_m + i I_m = (s + i t)^m`, the
//! derived Legendre functions `A_nm(u) = P_nm(u) / (1-u^2)^{m/2}` (with
//! `A'_nm = A_{n,m+1}`), and `q_n = (GM/r)(a/r)^n`, differentiating the
//! potential termwise gives
//!
//! ```text
//! a1 = sum (q_n/r) m A_nm E_nm        E = C R_{m-1} + S I_{m-1}
//! a2 = sum (q_n/r) m A_nm F_nm        F = S R_{m-1} - C I_{m-1}
//! a3 = sum (q_n/r) A_{n,m+1} D_nm     D = C R_m     + S I_m
//! a4 = sum (q_n/r) [(n+m+1) A_nm + u A_{n,m+1}] D_nm
//! a  = (a1 - s a4,  a2 - t a4,  a3 - u a4)
//! ```
//!
//! Fully normalized coefficients pair as `A_nm C_nm = Abar_nm Cbar_nm`;
//! the one cross-order product `A_{n,m+1} D_nm` needs the ratio
//! `Lambda_nm = N_nm / N_{n,m+1}` (spelled out in [`lambda`]). The n = 0
//! term of the sums reproduces the central `-GM r / |r|^3` exactly, so the
//! total field falls out of one code path.

use std::cell::{Cell, RefCell};
use std::sync::LazyLock;

use glam::DVec3;

use crate::propagation::bodies::{CentralBody, GravityField, PointMass};

/// The parsed packed-EGM2008 table (see `build.rs` for the format).
pub(crate) struct EgmTable {
    pub gm_m3_s2: f64,
    pub radius_m: f64,
    pub n_max: usize,
    c_bar: Vec<f64>,
    s_bar: Vec<f64>,
}

/// Triangular (n, m) -> flat index, n starting at 2 (matches the packer).
fn coefficient_index(n: usize, m: usize) -> usize {
    n * (n + 1) / 2 - 3 + m
}

impl EgmTable {
    fn parse(packed: &[u8]) -> Self {
        let read_u32 = |at: usize| u32::from_le_bytes(packed[at..at + 4].try_into().unwrap());
        let read_f64 = |at: usize| f64::from_le_bytes(packed[at..at + 8].try_into().unwrap());
        assert_eq!(&packed[..8], b"EGM2008\0", "packed EGM2008 magic");
        assert_eq!(read_u32(8), 1, "packed EGM2008 version");
        let n_max = read_u32(12) as usize;
        let gm_m3_s2 = read_f64(16);
        let radius_m = read_f64(24);
        let pairs = coefficient_index(n_max, n_max) + 1;
        assert_eq!(packed.len(), 40 + 16 * pairs, "packed EGM2008 size");
        let read_array = |base: usize| {
            (0..pairs)
                .map(|i| read_f64(base + 8 * i))
                .collect::<Vec<f64>>()
        };
        Self {
            gm_m3_s2,
            radius_m,
            n_max,
            c_bar: read_array(40),
            s_bar: read_array(40 + 8 * pairs),
        }
    }
}

/// The embedded table, parsed once per process.
pub(crate) static EGM2008: LazyLock<EgmTable> =
    LazyLock::new(|| EgmTable::parse(crate::data::EGM2008_PACKED));

/// IERS 2010 nominal (elastic) degree-2 Love numbers, k20/k21/k22.
const LOVE_K2: [f64; 3] = [0.295_25, 0.294_70, 0.298_01];
/// GM ratios perturber/Earth for the tide sum (DE440-class values).
const GM_RATIO_SUN: f64 = 332_946.048_7;
const GM_RATIO_MOON: f64 = 1.0 / 81.300_568_221_497_22;

/// EGM2008 truncated to `degree` x `order`, with optional degree-2 solid
/// tides. Registered for Earth; every other body stays point-mass.
pub(crate) struct Egm2008Gravity {
    table: &'static EgmTable,
    degree: usize,
    order: usize,
    solid_tides: bool,
    /// Frequency-independent tide deltas for (2,0) (2,1) (2,2):
    /// [dC20, dC21, dS21, dC22, dS22], refreshed per evaluation epoch
    /// through the time-dependence hook.
    tide_deltas: Cell<[f64; 5]>,
    scratch: RefCell<Scratch>,
}

#[derive(Default)]
struct Scratch {
    /// Normalized derived Legendre functions, triangular (n, m), n = 0..=N.
    a_bar: Vec<f64>,
    r_m: Vec<f64>,
    i_m: Vec<f64>,
}

/// Triangular index over the FULL table starting at n = 0 (scratch layout).
fn abar_index(n: usize, m: usize) -> usize {
    n * (n + 1) / 2 + m
}

/// `Lambda_nm = N_nm / N_{n,m+1}`: the normalization ratio carried by the
/// cross-order `A_{n,m+1} D_nm` products.
fn lambda(n: usize, m: usize) -> f64 {
    let nf = n as f64;
    let mf = m as f64;
    if m == 0 {
        (nf * (nf + 1.0) / 2.0).sqrt()
    } else {
        ((nf - mf) * (nf + mf + 1.0)).sqrt()
    }
}

impl Egm2008Gravity {
    pub(crate) fn new(degree: usize, order: usize, solid_tides: bool) -> Self {
        let table = &*EGM2008;
        Self {
            table,
            degree: degree.min(table.n_max),
            order: order.min(table.n_max),
            solid_tides,
            tide_deltas: Cell::new([0.0; 5]),
            scratch: RefCell::new(Scratch::default()),
        }
    }

    fn coefficients(&self, n: usize, m: usize) -> (f64, f64) {
        let index = coefficient_index(n, m);
        let mut c = self.table.c_bar[index];
        let mut s = self.table.s_bar[index];
        if n == 2 && self.solid_tides {
            let deltas = self.tide_deltas.get();
            match m {
                0 => c += deltas[0],
                1 => {
                    c += deltas[1];
                    s += deltas[2];
                }
                2 => {
                    c += deltas[3];
                    s += deltas[4];
                }
                _ => {}
            }
        }
        (c, s)
    }

    /// One perturber's contribution to the IERS frequency-independent
    /// degree-2 tide (eq. 6.6): `(k2m/5) (GM_j/GM_E) (a/r_j)^3
    /// P_bar_2m(sin phi_j) e^{-i m lambda_j}` accumulated into the deltas.
    fn accumulate_tide(deltas: &mut [f64; 5], gm_ratio: f64, body_fixed_m: DVec3, radius_m: f64) {
        let rho = body_fixed_m.length();
        let scale = gm_ratio * (radius_m / rho).powi(3) / 5.0;
        let (x, y, z) = (
            body_fixed_m.x / rho,
            body_fixed_m.y / rho,
            body_fixed_m.z / rho,
        );
        let sqrt15 = 15.0_f64.sqrt();
        // P_bar_20(z) = sqrt(5) (3 z^2 - 1) / 2, real.
        deltas[0] += LOVE_K2[0] * scale * 5.0_f64.sqrt() * (3.0 * z * z - 1.0) / 2.0;
        // P_bar_21 e^{-i lambda} = sqrt(15) z (x - i y).
        deltas[1] += LOVE_K2[1] * scale * sqrt15 * z * x;
        deltas[2] += LOVE_K2[1] * scale * sqrt15 * z * y;
        // P_bar_22 e^{-2 i lambda} = sqrt(15)/2 (x - i y)^2.
        deltas[3] += LOVE_K2[2] * scale * sqrt15 / 2.0 * (x * x - y * y);
        deltas[4] += LOVE_K2[2] * scale * sqrt15 * x * y;
    }
}

impl GravityField for Egm2008Gravity {
    fn acceleration_m_s2(&self, r_m: DVec3) -> DVec3 {
        let table = self.table;
        let n_max = self.degree;
        let r = r_m.length();
        let (s, t, u) = (r_m.x / r, r_m.y / r, r_m.z / r);

        let mut scratch = self.scratch.borrow_mut();
        let Scratch {
            a_bar,
            r_m: rm,
            i_m: im,
        } = &mut *scratch;
        a_bar.resize(abar_index(n_max + 1, n_max + 1) + 1, 0.0);
        rm.resize(n_max + 2, 0.0);
        im.resize(n_max + 2, 0.0);

        // (s + i t)^m.
        rm[0] = 1.0;
        im[0] = 0.0;
        for m in 1..=(n_max + 1) {
            rm[m] = s * rm[m - 1] - t * im[m - 1];
            im[m] = s * im[m - 1] + t * rm[m - 1];
        }

        // Normalized derived Legendre functions Abar_nm(u), computed to
        // degree n_max + 1 so the A_{n,m+1} products always resolve.
        let top = n_max + 1;
        a_bar[abar_index(0, 0)] = 1.0;
        if top >= 1 {
            a_bar[abar_index(1, 0)] = u * 3.0_f64.sqrt();
            a_bar[abar_index(1, 1)] = 3.0_f64.sqrt();
        }
        for n in 2..=top {
            let nf = n as f64;
            // Diagonal, then sub-diagonal.
            a_bar[abar_index(n, n)] =
                ((2.0 * nf + 1.0) / (2.0 * nf)).sqrt() * a_bar[abar_index(n - 1, n - 1)];
            a_bar[abar_index(n, n - 1)] =
                u * (2.0 * nf + 1.0).sqrt() * a_bar[abar_index(n - 1, n - 1)];
            for m in 0..=(n - 2) {
                let mf = m as f64;
                let c1 = ((2.0 * nf + 1.0) * (2.0 * nf - 1.0) / ((nf - mf) * (nf + mf))).sqrt();
                let c2 = ((2.0 * nf + 1.0) * (nf - mf - 1.0) * (nf + mf - 1.0)
                    / ((nf - mf) * (nf + mf) * (2.0 * nf - 3.0)))
                    .sqrt();
                a_bar[abar_index(n, m)] =
                    c1 * u * a_bar[abar_index(n - 1, m)] - c2 * a_bar[abar_index(n - 2, m)];
            }
        }

        // Pines accumulators over the harmonic degrees (n >= 2); the
        // central term is added analytically below.
        let (mut a1, mut a2, mut a3, mut a4) = (0.0, 0.0, 0.0, 0.0);
        let a_over_r = table.radius_m / r;
        let mut q_over_r = (table.gm_m3_s2 / r) * a_over_r * a_over_r / r; // q_2 / r
        for n in 2..=n_max {
            let nf = n as f64;
            for m in 0..=n.min(self.order) {
                let mf = m as f64;
                let (c, s_coeff) = self.coefficients(n, m);
                let d = c * rm[m] + s_coeff * im[m];
                let abar_nm = a_bar[abar_index(n, m)];
                if m > 0 {
                    let e = c * rm[m - 1] + s_coeff * im[m - 1];
                    let f = s_coeff * rm[m - 1] - c * im[m - 1];
                    a1 += q_over_r * mf * abar_nm * e;
                    a2 += q_over_r * mf * abar_nm * f;
                }
                let abar_next = if m < n {
                    lambda(n, m) * a_bar[abar_index(n, m + 1)]
                } else {
                    0.0
                };
                a3 += q_over_r * abar_next * d;
                a4 += q_over_r * ((nf + mf + 1.0) * abar_nm + u * abar_next) * d;
            }
            q_over_r *= a_over_r;
        }

        let central = -table.gm_m3_s2 / (r * r);
        DVec3::new(
            a1 - s * a4 + central * s,
            a2 - t * a4 + central * t,
            a3 - u * a4 + central * u,
        )
    }

    fn needs_body_fixed(&self) -> bool {
        true
    }

    fn mu_m3_s2(&self) -> f64 {
        self.table.gm_m3_s2
    }

    fn reference_radius_m(&self) -> f64 {
        self.table.radius_m
    }

    fn update_time_dependence(&self, sun_body_fixed_m: DVec3, moon_body_fixed_m: DVec3) {
        let mut deltas = [0.0; 5];
        let radius = self.table.radius_m;
        Self::accumulate_tide(&mut deltas, GM_RATIO_SUN, sun_body_fixed_m, radius);
        Self::accumulate_tide(&mut deltas, GM_RATIO_MOON, moon_body_fixed_m, radius);
        self.tide_deltas.set(deltas);
    }

    fn is_time_dependent(&self) -> bool {
        self.solid_tides
    }
}

/// The gravity registry (spec §4.0/§4.1): point-mass is the universal
/// default; Earth (NAIF 399) resolves to EGM2008 when a harmonic degree is
/// requested. Adding a body-specific model later is a new arm here, zero
/// changes elsewhere.
pub(crate) fn field_for(
    central: &CentralBody,
    degree: u16,
    order: u16,
    solid_tides: bool,
) -> Box<dyn GravityField> {
    if central.naif_id == 399 && degree >= 2 {
        Box::new(Egm2008Gravity::new(
            degree as usize,
            order as usize,
            solid_tides,
        ))
    } else {
        Box::new(PointMass {
            mu_m3_s2: central.mu_m3_s2,
            reference_radius_m: central.reference_radius_m,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent reference: the potential itself via the standard
    /// fully-normalized associated-Legendre recursion (a separate code
    /// path from the Pines derived functions), differentiated numerically.
    // The recursion reads three p_bar rows at once; index loops keep it
    // shaped like the math.
    #[allow(clippy::needless_range_loop)]
    fn potential(table: &EgmTable, degree: usize, order: usize, r_m: DVec3) -> f64 {
        let r = r_m.length();
        let u = r_m.z / r;
        let w = (1.0 - u * u).max(0.0).sqrt();
        let lambda_angle = r_m.y.atan2(r_m.x);

        // p_bar[n][m], fully normalized.
        let mut p_bar = vec![vec![0.0; degree + 1]; degree + 1];
        p_bar[0][0] = 1.0;
        if degree >= 1 {
            p_bar[1][0] = 3.0_f64.sqrt() * u;
            p_bar[1][1] = 3.0_f64.sqrt() * w;
        }
        for n in 2..=degree {
            let nf = n as f64;
            p_bar[n][n] = ((2.0 * nf + 1.0) / (2.0 * nf)).sqrt() * w * p_bar[n - 1][n - 1];
            p_bar[n][n - 1] = (2.0 * nf + 1.0).sqrt() * u * p_bar[n - 1][n - 1];
            for m in 0..=(n - 2) {
                let mf = m as f64;
                let c1 = ((2.0 * nf + 1.0) * (2.0 * nf - 1.0) / ((nf - mf) * (nf + mf))).sqrt();
                let c2 = ((2.0 * nf + 1.0) * (nf - mf - 1.0) * (nf + mf - 1.0)
                    / ((nf - mf) * (nf + mf) * (2.0 * nf - 3.0)))
                    .sqrt();
                p_bar[n][m] = c1 * u * p_bar[n - 1][m] - c2 * p_bar[n - 2][m];
            }
        }

        let mut v = table.gm_m3_s2 / r;
        let mut factor = (table.gm_m3_s2 / r) * (table.radius_m / r).powi(2);
        for n in 2..=degree {
            for m in 0..=n.min(order) {
                let index = coefficient_index(n, m);
                let (c, s) = (table.c_bar[index], table.s_bar[index]);
                let mf = m as f64;
                v += factor
                    * p_bar[n][m]
                    * (c * (mf * lambda_angle).cos() + s * (mf * lambda_angle).sin());
            }
            factor *= table.radius_m / r;
        }
        v
    }

    fn numerical_gradient(table: &EgmTable, degree: usize, order: usize, r_m: DVec3) -> DVec3 {
        let h = 0.5; // meters; V curvature is gentle at orbit scales
        let dv = |axis: DVec3| {
            (potential(table, degree, order, r_m + h * axis)
                - potential(table, degree, order, r_m - h * axis))
                / (2.0 * h)
        };
        DVec3::new(dv(DVec3::X), dv(DVec3::Y), dv(DVec3::Z))
    }

    /// Spec §7.13 (first half): truncated below degree 2 the model must
    /// reproduce point-mass gravity (with ITS OWN GM) to machine precision.
    #[test]
    fn degree_zero_is_point_mass() {
        let field = Egm2008Gravity::new(0, 0, false);
        let r = DVec3::new(5.1e6, -3.3e6, 2.2e6);
        let got = field.acceleration_m_s2(r);
        let want = -field.table.gm_m3_s2 / r.length().powi(3) * r;
        assert!(
            (got - want).length() < 1e-13 * want.length(),
            "{got:?} vs {want:?}"
        );
    }

    /// Degree-2, order-0 truncation against the textbook analytic J2
    /// acceleration - an absolute anchor for the (2,0) normalization
    /// (J2 = -sqrt(5) C_bar_20).
    #[test]
    fn degree_two_matches_analytic_j2() {
        let field = Egm2008Gravity::new(2, 0, false);
        let table = field.table;
        let j2 = -5.0_f64.sqrt() * table.c_bar[coefficient_index(2, 0)];
        assert!((j2 - 1.0826e-3).abs() < 1e-6, "J2 = {j2}");

        for r in [
            DVec3::new(7.1e6, 0.3e6, 1.9e6),
            DVec3::new(-2.0e6, 6.4e6, -3.1e6),
            DVec3::new(0.0, 0.0, 8.0e6), // on the polar axis
        ] {
            let got = field.acceleration_m_s2(r);
            let (radius, z) = (r.length(), r.z);
            let common =
                -1.5 * j2 * table.gm_m3_s2 * table.radius_m * table.radius_m / radius.powi(4);
            let zr2 = (z / radius) * (z / radius);
            let central = -table.gm_m3_s2 / radius.powi(3) * r;
            let want = central
                + common
                    * DVec3::new(
                        (1.0 - 5.0 * zr2) * r.x / radius,
                        (1.0 - 5.0 * zr2) * r.y / radius,
                        (3.0 - 5.0 * zr2) * z / radius,
                    );
            assert!(
                (got - want).length() < 1e-12 * want.length(),
                "at {r:?}: {got:?} vs {want:?}"
            );
        }
    }

    /// The full-field consistency gate: Pines acceleration against the
    /// numerical gradient of the independently-evaluated potential, all
    /// orders live, at LEO radius and near-pole geometry.
    #[test]
    fn acceleration_is_gradient_of_potential() {
        let field = Egm2008Gravity::new(8, 8, false);
        for r in [
            DVec3::new(6.9e6, 1.2e6, 0.8e6),
            DVec3::new(-1.5e6, -6.6e6, 2.4e6),
            DVec3::new(0.4e6, -0.2e6, 7.0e6), // 86.7 deg latitude
        ] {
            let got = field.acceleration_m_s2(r);
            let want = numerical_gradient(field.table, 8, 8, r);
            assert!(
                (got - want).length() < 1e-7 * want.length(),
                "at {r:?}: {got:?} vs {want:?} (|diff| {})",
                (got - want).length()
            );
        }
    }

    /// Spec §7.14: directly over both poles the evaluation must stay
    /// finite and match the m = 0 field (sectorials vanish there) - Pines
    /// has no polar singularity by construction.
    #[test]
    fn poles_are_regular() {
        let field = Egm2008Gravity::new(36, 36, false);
        for z in [7.0e6, -7.0e6] {
            let a = field.acceleration_m_s2(DVec3::new(0.0, 0.0, z));
            assert!(a.is_finite(), "pole acceleration {a:?}");
            // Pull is along -z_hat (toward the center), dominated by GM/r^2.
            let expected = field.table.gm_m3_s2 / (z * z);
            assert!(
                (a.length() - expected).abs() < 5e-3 * expected,
                "pole |a| {} vs {expected}",
                a.length()
            );
            assert!(a.z * z < 0.0, "pole pull points outward");
        }
    }

    /// Surface gravity lands in the observed envelope (equator ~9.78,
    /// pole ~9.83 m/s^2) - an absolute scale anchor.
    #[test]
    fn surface_gravity_envelope() {
        let field = Egm2008Gravity::new(8, 8, false);
        let equator = field
            .acceleration_m_s2(DVec3::new(6.378e6, 0.0, 0.0))
            .length();
        assert!((9.75..9.83).contains(&equator), "equatorial g = {equator}");
        let pole = field
            .acceleration_m_s2(DVec3::new(0.0, 0.0, 6.357e6))
            .length();
        assert!((9.79..9.88).contains(&pole), "polar g = {pole}");
        assert!(pole > equator, "gravity must increase toward the poles");
    }

    /// Solid-tide deltas: Moon on the x-axis at its mean distance plus the
    /// Sun at 1 AU produce the IERS-magnitude coefficient corrections
    /// (~1e-8), with the expected signs for an equatorial perturber.
    #[test]
    fn solid_tide_deltas_have_iers_magnitude() {
        let field = Egm2008Gravity::new(4, 4, true);
        field.update_time_dependence(
            DVec3::new(1.4959787e11, 0.0, 0.0),
            DVec3::new(3.844e8, 0.0, 0.0),
        );
        let deltas = field.tide_deltas.get();
        // Equatorial perturbers: dC20 < 0 (P_bar_20(0) < 0), dC22 > 0,
        // dS21 = dS22 = 0, dC21 = 0 (z = 0).
        assert!(
            (1e-9..1e-7).contains(&deltas[0].abs()) && deltas[0] < 0.0,
            "dC20 = {}",
            deltas[0]
        );
        assert!(
            (1e-9..1e-7).contains(&deltas[3]) && deltas[3] > 0.0,
            "dC22 = {}",
            deltas[3]
        );
        assert!(deltas[1].abs() < 1e-15 && deltas[2].abs() < 1e-15 && deltas[4].abs() < 1e-15);
    }
}
