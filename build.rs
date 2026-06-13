use std::fs;
use std::path::{Path, PathBuf};

use intel_tex_2::{RgbaSurface, bc7};

/// The atmosphere LUT bake, shared with nothing at runtime anymore: the
/// tables are baked here, written as KTX2, and the app only uploads them.
#[path = "src/globe/atmosphere.rs"]
mod atmosphere;

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

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    for asset in ASSETS {
        let source = download_if_missing(asset.url);
        transcode_if_missing(&source, asset.srgb, &out_dir);
    }

    bake_luts(&out_dir);
}

/// Bakes the atmosphere LUTs and writes them as uncompressed
/// `R16G16B16A16_SFLOAT` KTX2 files — the exact texels the runtime bake
/// used to produce, so moving the bake here changes nothing visually.
///
/// Unlike the BC7 transcode this runs on *every* script execution: the
/// bake is sub-second, and cargo reruns the script whenever
/// `src/globe/atmosphere.rs` (or `build.rs` itself) changes, so the
/// tables can never go stale after a constants tweak.
fn bake_luts(out_dir: &Path) {
    println!("cargo::rerun-if-changed=src/globe/atmosphere.rs");

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
        fs::write(&dest, ktx)
            .unwrap_or_else(|error| panic!("failed to write {dest:?}: {error}"));
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
/// `out_dir`, skipping the (slow) encode when the output already exists.
/// The source images never change, so existence is enough; delete the
/// `.ktx2` files from `OUT_DIR` (or the whole target dir) to force a
/// re-encode after changing encoder settings here.
fn transcode_if_missing(source: &Path, srgb: bool, out_dir: &Path) {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_else(|| panic!("no file stem in {source:?}"));
    let dest = out_dir.join(format!("{stem}.ktx2"));

    if dest.exists() {
        return;
    }

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
    // 8 for RGBA16F — 16 covers both).
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
