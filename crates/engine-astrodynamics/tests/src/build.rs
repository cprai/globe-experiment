use std::fs;
use std::path::{Path, PathBuf};

/// Satkit reference data downloaded once into `OUT_DIR` and embedded
/// verbatim (`include_bytes!`) - moved here from the parent crate when it
/// dropped satkit (refactor P4). The EOP snapshot spans 1962 onward (plus
/// a few months of predictions); delete a cached file to refresh it.
const EMBEDS: &[Embed] = &[
    // JPL DE440 ephemeris in satkit's native binary layout, ~98 MiB.
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
    // format matches `ierstable::from_bytes`. Required by satkit's full
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
    // match `earthgravity::Gravity::from_bytes`). Required by satkit's
    // numerical orbit propagator - its lazy default loader would otherwise
    // resolve a data dir at first propagation.
    Embed {
        url: "https://storage.googleapis.com/astrokit-astro-data/EGM96.gfc",
        limit: 64 * 1024 * 1024,
    },
    // JPL DE440 excerpt in SPICE .bsp layout (~31 MiB), for the astrodyn
    // reference side (`Ephemeris::from_bsp_bytes`). Same URL as one of the
    // parent crate's embeds; byte-identical Chebyshev content to the
    // satkit-format .440 file above, so the two reference stacks and the
    // crate all read the same integration.
    Embed {
        url: "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp",
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
