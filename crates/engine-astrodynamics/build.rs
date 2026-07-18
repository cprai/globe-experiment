use std::fs;
use std::path::{Path, PathBuf};

/// Satkit astronomical data downloaded once into `OUT_DIR` and embedded
/// verbatim (`include_bytes!`). All six files are fetched even though
/// ephemerides need only DE440: the crate will grow SGP4, frame-transform,
/// and propagation duties that need EOP, the IERS tables, and EGM96.
/// Deliberately duplicates the satkit rows of `crates/engine/build.rs`
/// until the engine migrates to this crate. The EOP snapshot spans 1962
/// onward (plus a few months of predictions); delete the cached file to
/// refresh.
const EMBEDS: &[Embed] = &[
    // JPL DE440 ephemeris, ~98 MiB; headroom for future DE versions.
    Embed {
        url: "https://ssd.jpl.nasa.gov/ftp/eph/planets/Linux/de440/linux_p1550p2650.440",
        limit: 256 * 1024 * 1024,
    },
    Embed {
        url: "https://celestrak.org/SpaceData/EOP-All.csv",
        limit: 64 * 1024 * 1024,
    },
    // IERS Conventions 2010 nutation/CIO tables (Tab5A/5B = CIP X/Y series,
    // Tab5D = CIO locator s), from satkit's own data bucket so the byte
    // format matches `ierstable::from_bytes`. Required by the full
    // (non-approx) GCRF<->ITRF transforms.
    Embed {
        url: "https://storage.googleapis.com/astrokit-astro-data/tab5.2a.txt",
        limit: 1024 * 1024,
    },
    Embed {
        url: "https://storage.googleapis.com/astrokit-astro-data/tab5.2b.txt",
        limit: 1024 * 1024,
    },
    Embed {
        url: "https://storage.googleapis.com/astrokit-astro-data/tab5.2d.txt",
        limit: 1024 * 1024,
    },
    // ICGEM EGM96 gravity coefficients, from satkit's bucket (format must
    // match `earthgravity::Gravity::from_bytes`). Required by the numerical
    // orbit propagator - its lazy default loader would otherwise resolve a
    // data dir at first propagation.
    Embed {
        url: "https://storage.googleapis.com/astrokit-astro-data/EGM96.gfc",
        limit: 64 * 1024 * 1024,
    },
];

struct Embed {
    url: &'static str,
    limit: u64,
}

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    for embed in EMBEDS {
        embed_verbatim(embed, &out_dir);
    }
}

/// Downloads `embed.url` into `OUT_DIR` under its own file name unless
/// already there; registers it with cargo so deleting it re-downloads.
fn embed_verbatim(embed: &Embed, out_dir: &Path) {
    let url = embed.url;
    let name = url.rsplit('/').next().expect("embed url has a file name");
    let dest = out_dir.join(name);

    println!("cargo::rerun-if-changed={}", dest.display());

    if dest.exists() {
        return;
    }

    let bytes = download(url, embed.limit);
    fs::write(&dest, bytes).unwrap_or_else(|error| panic!("failed to write {dest:?}: {error}"));
}

/// Fetches `url` fully into memory, capping the response body at `limit` bytes.
fn download(url: &str, limit: u64) -> Vec<u8> {
    let mut response = ureq::get(url)
        .call()
        .unwrap_or_else(|error| panic!("failed to download {url}: {error}"));
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .unwrap_or_else(|error| panic!("failed to read response of {url}: {error}"))
}
