use std::fs;
use std::path::{Path, PathBuf};

use intel_tex_2::{RgbaSurface, bc7};

/// Remote assets fetched into the gitignored assets directory, then
/// transcoded to BC7 in a KTX2 container in `OUT_DIR` so the runtime can
/// upload the bytes straight to the GPU with no decode step.
///
/// `srgb` marks color textures (encoded as `BC7_SRGB_BLOCK`); the normal
/// and specular maps are data and stay linear (`BC7_UNORM_BLOCK`).
const ASSETS: &[Asset] = &[
    Asset {
        url: "https://www.solarsystemscope.com/textures/download/8k_earth_daymap.jpg",
        srgb: true,
    },
    Asset {
        url: "https://www.solarsystemscope.com/textures/download/8k_earth_nightmap.jpg",
        srgb: true,
    },
    Asset {
        url: "https://www.solarsystemscope.com/textures/download/8k_earth_normal_map.tif",
        srgb: false,
    },
    Asset {
        url: "https://www.solarsystemscope.com/textures/download/8k_earth_specular_map.tif",
        srgb: false,
    },
    Asset {
        url: "https://www.solarsystemscope.com/textures/download/8k_stars_milky_way.jpg",
        srgb: true,
    },
];

struct Asset {
    url: &'static str,
    srgb: bool,
}

/// Generous cap per download; the largest texture is ~10 MB.
const DOWNLOAD_LIMIT: u64 = 100 * 1024 * 1024;

/// JPL Development Ephemeris (DE440) binary, downloaded into the gitignored
/// `data/` dir at build time and read at runtime by satkit's `jplephem` (the
/// app points `SATKIT_DATA` there). At ~98 MiB it is far too large to embed
/// in the binary like the textures, so it stays a side file the app loads.
const EPHEMERIS_URL: &str =
    "https://ssd.jpl.nasa.gov/ftp/eph/planets/Linux/de440/linux_p1550p2650.440";

/// The ephemeris is ~98 MiB; allow generous headroom for future DE versions.
const EPHEMERIS_LIMIT: u64 = 256 * 1024 * 1024;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    download_ephemeris();

    for asset in ASSETS {
        let source = download_if_missing(asset.url);
        transcode(&source, asset.srgb, &out_dir);
    }

    bake_luts(&out_dir);
}

/// Downloads the JPL ephemeris into `data/` unless it is already there, and
/// registers it with cargo so deleting it re-downloads on the next build. The
/// file is read at runtime (not embedded), so it lands in the project's
/// `data/` dir rather than `OUT_DIR`.
fn download_ephemeris() {
    let name = EPHEMERIS_URL
        .rsplit('/')
        .next()
        .expect("ephemeris url has a file name");
    let path = PathBuf::from(format!("data/{name}"));

    println!("cargo::rerun-if-changed={}", path.display());

    if path.exists() {
        return;
    }

    fs::create_dir_all("data")
        .unwrap_or_else(|error| panic!("failed to create data directory: {error}"));

    let mut response = ureq::get(EPHEMERIS_URL)
        .call()
        .unwrap_or_else(|error| panic!("failed to download {EPHEMERIS_URL}: {error}"));
    let bytes = response
        .body_mut()
        .with_config()
        .limit(EPHEMERIS_LIMIT)
        .read_to_vec()
        .unwrap_or_else(|error| panic!("failed to read response of {EPHEMERIS_URL}: {error}"));

    fs::write(&path, bytes).unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));
}

/// Bakes the atmosphere LUTs and writes them as uncompressed
/// `R16G16B16A16_SFLOAT` KTX2 files - the exact texels the old runtime
/// bake produced, so baking here changes nothing visually.
///
/// Unlike the BC7 transcode this runs on *every* script execution: the
/// bake is sub-second, and cargo always reruns the script when `build.rs`
/// (which now contains the bake) changes, so the tables can never go
/// stale after a constants tweak.
fn bake_luts(out_dir: &Path) {
    let luts = atmosphere::bake();

    let tables: [(&str, u32, u32, &[half::f16]); 3] = [
        (
            "transmittance",
            atmosphere::TRANSMITTANCE_WIDTH,
            atmosphere::TRANSMITTANCE_HEIGHT,
            &luts.transmittance,
        ),
        (
            "inscatter_rayleigh",
            atmosphere::INSCATTER_WIDTH,
            atmosphere::INSCATTER_HEIGHT,
            &luts.inscatter_rayleigh,
        ),
        (
            "inscatter_mie",
            atmosphere::INSCATTER_WIDTH,
            atmosphere::INSCATTER_HEIGHT,
            &luts.inscatter_mie,
        ),
    ];

    for (name, width, height, texels) in tables {
        let ktx = write_ktx2(
            ktx2::Format::R16G16B16A16_SFLOAT,
            width,
            height,
            bytemuck::cast_slice(texels),
        );
        let dest = out_dir.join(format!("{name}.ktx2"));
        fs::write(&dest, ktx).unwrap_or_else(|error| panic!("failed to write {dest:?}: {error}"));
    }
}

/// Downloads `url` into `assets/` unless the file is already there, and
/// registers it with cargo so deleting the file re-downloads it on the
/// next build.
fn download_if_missing(url: &str) -> PathBuf {
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or_else(|| panic!("no file name in asset url {url}"));
    let path = PathBuf::from(format!("assets/{name}"));

    println!("cargo::rerun-if-changed={}", path.display());

    if path.exists() {
        return path;
    }

    fs::create_dir_all("assets")
        .unwrap_or_else(|error| panic!("failed to create assets directory: {error}"));

    let mut response = ureq::get(url)
        .call()
        .unwrap_or_else(|error| panic!("failed to download {url}: {error}"));
    let bytes = response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .unwrap_or_else(|error| panic!("failed to read response of {url}: {error}"));

    fs::write(&path, bytes).unwrap_or_else(|error| panic!("failed to write {path:?}: {error}"));

    path
}

/// Decodes `source`, BC7-compresses it, and writes `<stem>.ktx2` into
/// `out_dir`. This runs unconditionally on every build-script execution;
/// cargo decides when that happens via the `cargo::rerun-if-changed` line
/// per asset in `download_if_missing` (plus build.rs itself). So a
/// no-change rebuild skips the script entirely and does no encoding, while
/// editing a texture on disk - or the encoder settings in this script -
/// reruns the script and refreshes the output.
fn transcode(source: &Path, srgb: bool, out_dir: &Path) {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_else(|| panic!("no file stem in {source:?}"));
    let dest = out_dir.join(format!("{stem}.ktx2"));

    let image = image::open(source)
        .unwrap_or_else(|error| panic!("decode {source:?}: {error}"))
        .to_rgba8();
    let (width, height) = image.dimensions();
    assert!(
        width % 4 == 0 && height % 4 == 0,
        "{source:?} is {width}x{height}; BC7 needs multiple-of-4 dimensions"
    );

    let surface = RgbaSurface {
        data: image.as_raw(),
        width,
        height,
        stride: width * 4,
    };
    // All textures are opaque. The basic profile is slower than the
    // ultra-fast ones but this runs once per texture and then caches.
    let blocks = bc7::compress_blocks(&bc7::opaque_basic_settings(), &surface);

    let format = if srgb {
        ktx2::Format::BC7_SRGB_BLOCK
    } else {
        ktx2::Format::BC7_UNORM_BLOCK
    };

    let ktx = write_ktx2(format, width, height, &blocks);
    fs::write(&dest, ktx).unwrap_or_else(|error| panic!("failed to write {dest:?}: {error}"));
}

/// Serializes a single-level 2D texture as a KTX2 file: 80-byte header,
/// one level-index entry, a basic data format descriptor, then the raw
/// block data (no supercompression), 16-byte aligned per the spec.
fn write_ktx2(format: ktx2::Format, width: u32, height: u32, blocks: &[u8]) -> Vec<u8> {
    let (basic_dfd, type_size) = ktx2::dfd::Basic::from_format(format)
        .unwrap_or_else(|error| panic!("no DFD for {format:?}: {error:?}"));
    let dfd_block = ktx2::dfd::Block::Basic(basic_dfd);

    let dfd_offset = ktx2::Header::LENGTH + ktx2::LevelIndex::LENGTH;
    // The DFD section is a 4-byte total-length field plus the block.
    let dfd_length = 4 + dfd_block.serialized_length();
    // Level data must be aligned to the texel block size (16 for BC7,
    // 8 for RGBA16F - 16 covers both).
    let data_offset = (dfd_offset + dfd_length).next_multiple_of(16);

    let header = ktx2::Header {
        format: Some(format),
        type_size,
        pixel_width: width,
        pixel_height: height,
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: 1,
        supercompression_scheme: None,
        index: ktx2::Index {
            dfd_byte_offset: dfd_offset as u32,
            dfd_byte_length: dfd_length as u32,
            kvd_byte_offset: 0,
            kvd_byte_length: 0,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };

    let level = ktx2::LevelIndex {
        byte_offset: data_offset as u64,
        byte_length: blocks.len() as u64,
        uncompressed_byte_length: blocks.len() as u64,
    };

    let mut out = Vec::with_capacity(data_offset + blocks.len());
    out.extend_from_slice(&header.as_bytes());
    out.extend_from_slice(&level.as_bytes());
    out.extend_from_slice(&(dfd_length as u32).to_le_bytes());
    let block_start = out.len();
    out.resize(block_start + dfd_block.serialized_length(), 0);
    dfd_block.to_bytes(&mut out[block_start..]);
    out.resize(data_offset, 0);
    out.extend_from_slice(blocks);

    out
}

mod atmosphere {
    //! Precomputed atmosphere lookup tables, after Hillaire 2020 ("A
    //! Scalable and Production Ready Sky and Atmosphere Rendering
    //! Technique") with Earth's standard medium: Rayleigh and Mie
    //! scattering plus an ozone absorption layer.
    //!
    //! This code is build-time only - the app uploads the baked KTX2
    //! tables and never touches this math at runtime - so it lives in the
    //! build script rather than the runtime crate.
    //!
    //! Two kinds of LUT are baked on the CPU:
    //!
    //! - Transmittance: fraction of sunlight surviving from a point to the
    //!   top of the atmosphere, parameterized by (altitude, sun zenith
    //!   cosine).
    //! - Inscatter: the Rayleigh and Mie single-scattering integrals along
    //!   a full view ray. Because the scene is a perfect sphere seen from
    //!   outside, a ray is fully described by its impact parameter (closest
    //!   approach to the planet center) plus the sun angle at a reference
    //!   point, so the per-pixel raymarch collapses into a 2D table. The
    //!   only approximation is the sun's tilt *along* the ray, which is
    //!   assumed perpendicular.
    //!
    //! The constants here must stay in sync with their WGSL twins in
    //! `shaders/globe.wgsl`. All lengths are kilometers; all coefficients
    //! are per kilometer.

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
        let (inscatter_rayleigh, inscatter_mie) = bake_inscatter(&transmittance);

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
            RAYLEIGH_SCATTERING[i] * rayleigh + MIE_EXTINCTION * mie + OZONE_ABSORPTION[i] * ozone
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

        let mut table = Vec::with_capacity((TRANSMITTANCE_WIDTH * TRANSMITTANCE_HEIGHT) as usize);

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
                    ((h_top * h_top - rho * rho - d * d) / (2.0 * r * d)).clamp(-1.0, 1.0)
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
            let top = table[y0 * w + x0][c] * (1.0 - tx) + table[y0 * w + x1][c] * tx;
            let bottom = table[y1 * w + x0][c] * (1.0 - tx) + table[y1 * w + x1][c] * tx;
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
    fn bake_inscatter(transmittance: &[[f32; 3]]) -> (Vec<f16>, Vec<f16>) {
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
            let ref_point = if hits_ground { [t_exit, b] } else { [0.0, b] };
            let ref_r = (ref_point[0] * ref_point[0] + ref_point[1] * ref_point[1])
                .sqrt()
                .max(1e-3);
            let ref_hat = [ref_point[0] / ref_r, ref_point[1] / ref_r];

            for i in 0..INSCATTER_WIDTH {
                let mu_ref = 2.0 * (i as f32 + 0.5) / INSCATTER_WIDTH as f32 - 1.0;

                let dt = (t_exit - t_entry) / INSCATTER_STEPS as f32;
                let mut view_trans = [1.0f32; 3];
                let mut sum_rayleigh = [0.0f32; 3];
                let mut sum_mie = [0.0f32; 3];

                for s in 0..INSCATTER_STEPS {
                    let t = t_entry + (s as f32 + 0.5) * dt;
                    let r = (t * t + b * b).sqrt();
                    let h = (r - rp).max(0.0);

                    let mu_sun = mu_ref * (t * ref_hat[0] + b * ref_hat[1]) / r;
                    let t_sun = sample_transmittance(transmittance, r, mu_sun);

                    let (scatter_r, scatter_m) = scattering(h);
                    let sigma_t = extinction(h);

                    for c in 0..3 {
                        // Analytic integration of the inscatter across the
                        // step (Hillaire 2020, eq. 11).
                        let step_trans = (-sigma_t[c] * dt).exp();
                        let integ =
                            view_trans[c] * t_sun[c] * (1.0 - step_trans) / sigma_t[c].max(1e-6);

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
}
