use std::fs;
use std::path::Path;

/// Remote assets fetched into the gitignored assets directory, each
/// stored under its URL's file name.
const ASSET_URLS: &[&str] = &[
    "https://www.solarsystemscope.com/textures/download/8k_earth_daymap.jpg",
    "https://www.solarsystemscope.com/textures/download/8k_earth_nightmap.jpg",
    "https://www.solarsystemscope.com/textures/download/8k_earth_normal_map.tif",
    "https://www.solarsystemscope.com/textures/download/8k_earth_specular_map.tif",
    "https://www.solarsystemscope.com/textures/download/8k_stars_milky_way.jpg",
];

/// Generous cap per download; the largest texture is ~10 MB.
const DOWNLOAD_LIMIT: u64 = 100 * 1024 * 1024;

fn main() {
    for url in ASSET_URLS {
        download_if_missing(url);
    }
}

/// Downloads `url` into `assets/` unless the file is already there, and
/// registers it with cargo so deleting the file re-downloads it on the
/// next build.
fn download_if_missing(url: &str) {
    let name = url
        .rsplit('/')
        .next()
        .unwrap_or_else(|| panic!("no file name in asset url {url}"));
    let path = format!("assets/{name}");

    println!("cargo::rerun-if-changed={path}");

    if Path::new(&path).exists() {
        return;
    }

    fs::create_dir_all("assets").unwrap_or_else(|error| {
        panic!("failed to create assets directory: {error}")
    });

    let mut response = ureq::get(url)
        .call()
        .unwrap_or_else(|error| panic!("failed to download {url}: {error}"));
    let bytes = response
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .unwrap_or_else(|error| {
            panic!("failed to read response of {url}: {error}")
        });

    fs::write(&path, bytes)
        .unwrap_or_else(|error| panic!("failed to write {path}: {error}"));
}
