//! Transmittance lookup table for the atmosphere, after Hillaire 2020
//! ("A Scalable and Production Ready Sky and Atmosphere Rendering
//! Technique") with Earth's standard medium: Rayleigh and Mie scattering
//! plus an ozone absorption layer.
//!
//! The constants here must stay in sync with their WGSL twins in
//! `shaders/globe.wgsl`. All lengths are kilometers; all coefficients are
//! per kilometer.

use half::f16;

pub const PLANET_RADIUS_KM: f32 = 6360.0;
pub const ATMOSPHERE_TOP_KM: f32 = 6460.0;

pub const TRANSMITTANCE_WIDTH: u32 = 256;
pub const TRANSMITTANCE_HEIGHT: u32 = 64;

const RAYLEIGH_SCATTERING: [f32; 3] = [5.802e-3, 13.558e-3, 33.1e-3];
const RAYLEIGH_SCALE_HEIGHT: f32 = 8.0;
const MIE_EXTINCTION: f32 = 4.40e-3;
const MIE_SCALE_HEIGHT: f32 = 1.2;
const OZONE_ABSORPTION: [f32; 3] = [0.650e-3, 1.881e-3, 0.085e-3];

const INTEGRATION_STEPS: u32 = 40;

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

/// Builds the transmittance LUT: for a point at radius `r` and a ray with
/// cosine `mu` to the zenith, the fraction of light surviving the trip to
/// the top of the atmosphere. Texels are RGBA f16, parameterized with the
/// Bruneton mapping (x: distance-to-top fraction, y: altitude fraction),
/// which spends resolution near the horizon where transmittance changes
/// fastest.
pub fn transmittance_lut() -> Vec<f16> {
    let rp = PLANET_RADIUS_KM;
    let ra = ATMOSPHERE_TOP_KM;
    // Distance to the atmosphere top along the horizon, from the ground.
    let h_top = (ra * ra - rp * rp).sqrt();

    let mut texels = Vec::with_capacity(
        (TRANSMITTANCE_WIDTH * TRANSMITTANCE_HEIGHT * 4) as usize,
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

            // Integrate extinction along the ray.
            let dt = d / INTEGRATION_STEPS as f32;
            let mut optical_depth = [0.0f32; 3];

            for s in 0..INTEGRATION_STEPS {
                let t = (s as f32 + 0.5) * dt;
                let r_t = (r * r + t * t + 2.0 * r * mu * t).sqrt();
                let h = (r_t - rp).max(0.0);
                let sigma_t = extinction(h);

                for c in 0..3 {
                    optical_depth[c] += sigma_t[c] * dt;
                }
            }

            for c in 0..3 {
                texels.push(f16::from_f32((-optical_depth[c]).exp()));
            }
            texels.push(f16::from_f32(1.0));
        }
    }

    texels
}
