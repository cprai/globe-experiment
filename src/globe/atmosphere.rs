//! Precomputed atmosphere lookup tables, after Hillaire 2020 ("A Scalable
//! and Production Ready Sky and Atmosphere Rendering Technique") with
//! Earth's standard medium: Rayleigh and Mie scattering plus an ozone
//! absorption layer.
//!
//! Two kinds of LUT are baked on the CPU at startup:
//!
//! - Transmittance: fraction of sunlight surviving from a point to the top
//!   of the atmosphere, parameterized by (altitude, sun zenith cosine).
//! - Inscatter: the Rayleigh and Mie single-scattering integrals along a
//!   full view ray. Because the scene is a perfect sphere seen from
//!   outside, a ray is fully described by its impact parameter (closest
//!   approach to the planet center) plus the sun angle at a reference
//!   point, so the per-pixel raymarch collapses into a 2D table. The only
//!   approximation is the sun's tilt *along* the ray, which is assumed
//!   perpendicular.
//!
//! The constants here must stay in sync with their WGSL twins in
//! `shaders/globe.wgsl`. All lengths are kilometers; all coefficients are
//! per kilometer.

use half::f16;

pub const PLANET_RADIUS_KM: f32 = 6360.0;
pub const ATMOSPHERE_TOP_KM: f32 = 6460.0;

pub const TRANSMITTANCE_WIDTH: u32 = 256;
pub const TRANSMITTANCE_HEIGHT: u32 = 64;

/// Inscatter LUT axes: x is the sun cosine at the reference point,
/// y is the impact parameter (split mapping: lower half ground-hitting
/// rays, upper half limb rays).
pub const INSCATTER_WIDTH: u32 = 256;
pub const INSCATTER_HEIGHT: u32 = 128;

const RAYLEIGH_SCATTERING: [f32; 3] = [5.802e-3, 13.558e-3, 33.1e-3];
const RAYLEIGH_SCALE_HEIGHT: f32 = 8.0;
const MIE_SCATTERING: f32 = 3.996e-3;
const MIE_EXTINCTION: f32 = 4.40e-3;
const MIE_SCALE_HEIGHT: f32 = 1.2;
const OZONE_ABSORPTION: [f32; 3] = [0.650e-3, 1.881e-3, 0.085e-3];

const TRANSMITTANCE_STEPS: u32 = 40;
const INSCATTER_STEPS: u32 = 32;

pub struct Luts {
    /// RGBA f16, `TRANSMITTANCE_WIDTH x TRANSMITTANCE_HEIGHT`.
    pub transmittance: Vec<f16>,
    /// RGBA f16, `INSCATTER_WIDTH x INSCATTER_HEIGHT`.
    pub inscatter_rayleigh: Vec<f16>,
    /// RGBA f16, `INSCATTER_WIDTH x INSCATTER_HEIGHT`.
    pub inscatter_mie: Vec<f16>,
}

pub fn bake() -> Luts {
    let transmittance = bake_transmittance();
    let (inscatter_rayleigh, inscatter_mie) =
        bake_inscatter(&transmittance);

    Luts {
        transmittance: to_f16_texels(&transmittance),
        inscatter_rayleigh,
        inscatter_mie,
    }
}

/// Extinction coefficient of the medium at altitude `h` km.
fn extinction(h: f32) -> [f32; 3] {
    let rayleigh = (-h / RAYLEIGH_SCALE_HEIGHT).exp();
    let mie = (-h / MIE_SCALE_HEIGHT).exp();
    // Ozone concentration is a tent function peaking at 25 km.
    let ozone = (1.0 - (h - 25.0).abs() / 15.0).max(0.0);

    std::array::from_fn(|i| {
        RAYLEIGH_SCATTERING[i] * rayleigh
            + MIE_EXTINCTION * mie
            + OZONE_ABSORPTION[i] * ozone
    })
}

/// Rayleigh and Mie scattering coefficients at altitude `h` km.
fn scattering(h: f32) -> ([f32; 3], f32) {
    let rayleigh = (-h / RAYLEIGH_SCALE_HEIGHT).exp();
    let mie = (-h / MIE_SCALE_HEIGHT).exp();

    (
        RAYLEIGH_SCATTERING.map(|s| s * rayleigh),
        MIE_SCATTERING * mie,
    )
}

/// Builds the transmittance table with the Bruneton mapping
/// (x: distance-to-top fraction, y: altitude fraction), which spends
/// resolution near the horizon where transmittance changes fastest.
fn bake_transmittance() -> Vec<[f32; 3]> {
    let rp = PLANET_RADIUS_KM;
    let ra = ATMOSPHERE_TOP_KM;
    // Distance to the atmosphere top along the horizon, from the ground.
    let h_top = (ra * ra - rp * rp).sqrt();

    let mut table = Vec::with_capacity(
        (TRANSMITTANCE_WIDTH * TRANSMITTANCE_HEIGHT) as usize,
    );

    for j in 0..TRANSMITTANCE_HEIGHT {
        let x_r = (j as f32 + 0.5) / TRANSMITTANCE_HEIGHT as f32;
        let rho = x_r * h_top;
        let r = (rho * rho + rp * rp).sqrt();

        for i in 0..TRANSMITTANCE_WIDTH {
            let x_mu = (i as f32 + 0.5) / TRANSMITTANCE_WIDTH as f32;

            // Invert the mapping: recover the ray length to the top of
            // the atmosphere, then the ray's zenith cosine.
            let d_min = ra - r;
            let d_max = rho + h_top;
            let d = d_min + x_mu * (d_max - d_min);
            let mu = if d <= 0.0 {
                1.0
            } else {
                ((h_top * h_top - rho * rho - d * d) / (2.0 * r * d))
                    .clamp(-1.0, 1.0)
            };

            let dt = d / TRANSMITTANCE_STEPS as f32;
            let mut optical_depth = [0.0f32; 3];

            for s in 0..TRANSMITTANCE_STEPS {
                let t = (s as f32 + 0.5) * dt;
                let r_t = (r * r + t * t + 2.0 * r * mu * t).sqrt();
                let sigma_t = extinction((r_t - rp).max(0.0));

                for c in 0..3 {
                    optical_depth[c] += sigma_t[c] * dt;
                }
            }

            table.push(optical_depth.map(|d| (-d).exp()));
        }
    }

    table
}

/// Samples the baked transmittance table toward the sun, mirroring the
/// WGSL `sun_transmittance` (horizon shadow plus the Bruneton mapping).
fn sample_transmittance(table: &[[f32; 3]], r: f32, mu: f32) -> [f32; 3] {
    let rp = PLANET_RADIUS_KM;
    let ra = ATMOSPHERE_TOP_KM;

    let sin_horizon = rp / r;
    let cos_horizon = -(1.0 - sin_horizon * sin_horizon).max(0.0).sqrt();
    if mu < cos_horizon {
        return [0.0; 3];
    }

    let h_top = (ra * ra - rp * rp).sqrt();
    let rho = (r * r - rp * rp).max(0.0).sqrt();
    let d = -r * mu + (r * r * (mu * mu - 1.0) + ra * ra).max(0.0).sqrt();
    let d_min = ra - r;
    let d_max = rho + h_top;

    let x_mu = ((d - d_min) / (d_max - d_min).max(1e-4)).clamp(0.0, 1.0);
    let x_r = (rho / h_top).clamp(0.0, 1.0);

    // Bilinear lookup at texel centers.
    let (w, h) = (TRANSMITTANCE_WIDTH as usize, TRANSMITTANCE_HEIGHT as usize);
    let fx = (x_mu * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (x_r * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);

    std::array::from_fn(|c| {
        let top = table[y0 * w + x0][c] * (1.0 - tx)
            + table[y0 * w + x1][c] * tx;
        let bottom = table[y1 * w + x0][c] * (1.0 - tx)
            + table[y1 * w + x1][c] * tx;
        top * (1.0 - ty) + bottom * ty
    })
}

/// Bakes the Rayleigh and Mie inscatter sums (without phase functions,
/// which are constant per ray and applied at draw time).
///
/// Canonical geometry: the ray runs along +x with closest approach `b`
/// at (0, b, 0). The sun is placed at the requested cosine against the
/// reference point's zenith, tilted out of the ray plane, so the sun
/// cosine elsewhere on the ray follows the sphere's geometry:
/// `mu(t) = mu_ref * dot(p_hat, r_hat_ref)`.
fn bake_inscatter(
    transmittance: &[[f32; 3]],
) -> (Vec<f16>, Vec<f16>) {
    let rp = PLANET_RADIUS_KM;
    let ra = ATMOSPHERE_TOP_KM;

    let texels = (INSCATTER_WIDTH * INSCATTER_HEIGHT * 4) as usize;
    let mut rayleigh_lut = Vec::with_capacity(texels);
    let mut mie_lut = Vec::with_capacity(texels);

    for j in 0..INSCATTER_HEIGHT {
        let v = (j as f32 + 0.5) / INSCATTER_HEIGHT as f32;

        // Split mapping: lower half are rays that hit the ground,
        // upper half graze the limb.
        let (b, hits_ground) = if v < 0.5 {
            ((v * 2.0) * rp, true)
        } else {
            (rp + (v - 0.5) * 2.0 * (ra - rp), false)
        };

        let t_entry = -(ra * ra - b * b).max(0.0).sqrt();
        let t_exit = if hits_ground {
            -(rp * rp - b * b).max(0.0).sqrt()
        } else {
            (ra * ra - b * b).max(0.0).sqrt()
        };

        // Reference point: ground hit, or closest approach for limb rays.
        let ref_point = if hits_ground {
            [t_exit, b]
        } else {
            [0.0, b]
        };
        let ref_r = (ref_point[0] * ref_point[0]
            + ref_point[1] * ref_point[1])
            .sqrt()
            .max(1e-3);
        let ref_hat = [ref_point[0] / ref_r, ref_point[1] / ref_r];

        for i in 0..INSCATTER_WIDTH {
            let mu_ref =
                2.0 * (i as f32 + 0.5) / INSCATTER_WIDTH as f32 - 1.0;

            let dt = (t_exit - t_entry) / INSCATTER_STEPS as f32;
            let mut view_trans = [1.0f32; 3];
            let mut sum_rayleigh = [0.0f32; 3];
            let mut sum_mie = [0.0f32; 3];

            for s in 0..INSCATTER_STEPS {
                let t = t_entry + (s as f32 + 0.5) * dt;
                let r = (t * t + b * b).sqrt();
                let h = (r - rp).max(0.0);

                let mu_sun =
                    mu_ref * (t * ref_hat[0] + b * ref_hat[1]) / r;
                let t_sun = sample_transmittance(transmittance, r, mu_sun);

                let (scatter_r, scatter_m) = scattering(h);
                let sigma_t = extinction(h);

                for c in 0..3 {
                    // Analytic integration of the inscatter across the
                    // step (Hillaire 2020, eq. 11).
                    let step_trans = (-sigma_t[c] * dt).exp();
                    let integ = view_trans[c] * t_sun[c]
                        * (1.0 - step_trans)
                        / sigma_t[c].max(1e-6);

                    sum_rayleigh[c] += scatter_r[c] * integ;
                    sum_mie[c] += scatter_m * integ;
                    view_trans[c] *= step_trans;
                }
            }

            for c in 0..3 {
                rayleigh_lut.push(f16::from_f32(sum_rayleigh[c]));
                mie_lut.push(f16::from_f32(sum_mie[c]));
            }
            rayleigh_lut.push(f16::from_f32(1.0));
            mie_lut.push(f16::from_f32(1.0));
        }
    }

    (rayleigh_lut, mie_lut)
}

fn to_f16_texels(table: &[[f32; 3]]) -> Vec<f16> {
    let mut texels = Vec::with_capacity(table.len() * 4);

    for rgb in table {
        for &c in rgb {
            texels.push(f16::from_f32(c));
        }
        texels.push(f16::from_f32(1.0));
    }

    texels
}
