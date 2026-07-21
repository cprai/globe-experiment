use std::fs;
use std::path::{Path, PathBuf};

/// Astronomical data downloaded once into `OUT_DIR` and embedded verbatim
/// (`include_bytes!`); delete a cached file to refresh it. (The satkit-era
/// reference assets live in the harness's own build script now -
/// `tests/src/build.rs`.)
const EMBEDS: &[Embed] = &[
    // JPL DE440 excerpt, 1849-2150, ~31 MiB. Byte-identical to de440.bsp
    // inside that span (it is an excerpt); both are embedded deliberately -
    // the Almanac load order makes this one serve the overlap and the full
    // file serve 1550-2650 outside it (see data.rs).
    Embed {
        url: "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440s.bsp",
        limit: 64 * 1024 * 1024,
    },
    // Full JPL DE440, 1550-2650, ~114 MiB.
    Embed {
        url: "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/spk/planets/de440.bsp",
        limit: 256 * 1024 * 1024,
    },
    // High-precision binary Earth PCK (ITRF93, frame class 3000), 1962-2125:
    // the only single kernel covering every past scene. NAIF renames it
    // roughly annually (`earth_1962_<lastdatum>_2125_combined.bpc`); to bump,
    // update this URL and delete the cached file. A stale kernel only
    // degrades the low-accuracy predict tail past its last datum - old
    // scenes keep full accuracy.
    Embed {
        url: "https://naif.jpl.nasa.gov/pub/naif/generic_kernels/pck/earth_1962_250826_2125_combined.bpc",
        limit: 64 * 1024 * 1024,
    },
    // anise planetary-constants kernel (per-body mu/radii/flattening). The
    // nyxspace mirror is HTTP-only (its HTTPS cert mismatches); the content
    // is pinned at context build by its raw-byte crc32 instead (data.rs).
    Embed {
        url: "http://public-data.nyxspace.com/anise/v0.10/pck11.pca",
        limit: 1024 * 1024,
    },
    // CelesTrak space weather (1957 -> present + predictions): F10.7 daily
    // and centered 81-day mean, Ap daily and 3-hourly - every NRLMSISE-00
    // input, plus the OBS/PRD flag for the observed-only policy. A stale
    // snapshot makes drag FAIL LOUDLY at epochs past its last observed
    // datum (owner-accepted); delete the cached file to refresh.
    Embed {
        url: "https://celestrak.org/SpaceData/SW-All.csv",
        limit: 64 * 1024 * 1024,
    },
];

struct Embed {
    url: &'static str,
    limit: u64,
}

/// ICGEM EGM2008, the only transformed (non-verbatim) asset. The source
/// `.gfc` is 252 MB of text, but its records are sorted ascending by degree,
/// so the download streams and aborts after degree [`EGM2008_MAX_DEGREE`]
/// (~8 MB transferred), then packs the fully-normalized coefficients into a
/// small crate-owned binary (see [`embed_egm2008_packed`]). The hash URL has
/// no permanence guarantee (it survived ICGEM's domain move); if it 404s,
/// re-locate EGM2008 in the icgem.gfz.de model listing.
const EGM2008_URL: &str = "https://icgem.gfz.de/getmodel/gfc/c50128797a9cb62e936337c890e4425f03f0461d7329b09a8cc8561504465340/EGM2008.gfc";
const EGM2008_MAX_DEGREE: usize = 360;
const EGM2008_DEST: &str = "egm2008_n360.le64";
/// Safety cap on the streamed read: if the ascending-degree assumption ever
/// breaks (no early abort), this stops the 252 MB download partway and the
/// resulting parse failure points at the format drift.
const EGM2008_STREAM_LIMIT: u64 = 64 * 1024 * 1024;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));

    for embed in EMBEDS {
        embed_verbatim(embed, &out_dir);
    }
    embed_egm2008_packed(&out_dir);
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

/// Streams the ICGEM EGM2008 `.gfc`, truncates at degree
/// [`EGM2008_MAX_DEGREE`], and packs the coefficients into `OUT_DIR` as
/// [`EGM2008_DEST`]: a 40-byte header (`b"EGM2008\0"` magic, u32 version = 1,
/// u32 n_max, f64 GM, f64 a - the model's OWN defining constants, which the
/// gravity evaluation must use instead of any canonical mu - u64 reserved)
/// followed by the C-bar then S-bar triangular arrays (n = 2..=n_max,
/// m = 0..=n, degree-major), little-endian f64.
fn embed_egm2008_packed(out_dir: &Path) {
    use std::io::{BufRead, BufReader};

    let dest = out_dir.join(EGM2008_DEST);
    println!("cargo::rerun-if-changed={}", dest.display());
    if dest.exists() {
        return;
    }

    let mut response = ureq::get(EGM2008_URL)
        .call()
        .unwrap_or_else(|error| panic!("failed to download {EGM2008_URL}: {error}"));
    let reader = BufReader::new(
        response
            .body_mut()
            .with_config()
            .limit(EGM2008_STREAM_LIMIT)
            .reader(),
    );

    // Triangular (n, m) -> flat index, n starting at 2 (degree 0 is the
    // point-mass term, degree 1 vanishes about the center of mass).
    let index = |n: usize, m: usize| n * (n + 1) / 2 - 3 + m;
    let len = index(EGM2008_MAX_DEGREE, EGM2008_MAX_DEGREE) + 1;
    let mut c_bar = vec![f64::NAN; len];
    let mut s_bar = vec![f64::NAN; len];
    let mut gm = None;
    let mut radius = None;
    let mut in_header = true;

    for line in reader.lines() {
        let line = line.unwrap_or_else(|error| panic!("failed streaming {EGM2008_URL}: {error}"));
        let fields: Vec<&str> = line.split_whitespace().collect();
        if in_header {
            match fields.as_slice() {
                ["earth_gravity_constant", value, ..] => gm = Some(icgem_f64(value)),
                ["radius", value, ..] => radius = Some(icgem_f64(value)),
                ["tide_system", value, ..] => assert_eq!(*value, "tide_free", "EGM2008 header"),
                ["norm", value, ..] => assert_eq!(*value, "fully_normalized", "EGM2008 header"),
                ["end_of_head", ..] => in_header = false,
                _ => {}
            }
            continue;
        }
        let ["gfc", n, m, c, s, ..] = fields.as_slice() else {
            continue;
        };
        let n: usize = n.parse().expect("EGM2008 degree");
        if n > EGM2008_MAX_DEGREE {
            break; // Records ascend by degree; dropping the reader aborts the transfer.
        }
        if n < 2 {
            continue;
        }
        let m: usize = m.parse().expect("EGM2008 order");
        c_bar[index(n, m)] = icgem_f64(c);
        s_bar[index(n, m)] = icgem_f64(s);
    }

    // The pinned hash URL must keep resolving to the same model: check its
    // defining constants and that truncation left no hole.
    let gm = gm.expect("EGM2008 header carries earth_gravity_constant");
    let radius = radius.expect("EGM2008 header carries radius");
    assert!(
        (gm - 3.986_004_415e14).abs() < 1.0,
        "EGM2008 GM drifted: {gm}"
    );
    assert!(
        (radius - 6_378_136.3).abs() < 1e-3,
        "EGM2008 radius drifted: {radius}"
    );
    let holes = c_bar.iter().filter(|value| value.is_nan()).count();
    assert_eq!(
        holes, 0,
        "EGM2008 truncation left {holes} unfilled coefficients"
    );

    let mut packed = Vec::with_capacity(40 + 16 * len);
    packed.extend_from_slice(b"EGM2008\0");
    packed.extend_from_slice(&1u32.to_le_bytes());
    packed.extend_from_slice(&(EGM2008_MAX_DEGREE as u32).to_le_bytes());
    packed.extend_from_slice(&gm.to_le_bytes());
    packed.extend_from_slice(&radius.to_le_bytes());
    packed.extend_from_slice(&0u64.to_le_bytes());
    for value in c_bar.iter().chain(s_bar.iter()) {
        packed.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(&dest, packed).unwrap_or_else(|error| panic!("failed to write {dest:?}: {error}"));
}

/// Parses an ICGEM float, normalizing Fortran `D` exponents (`0.25D-06`).
fn icgem_f64(field: &str) -> f64 {
    field
        .replace(['D', 'd'], "E")
        .parse()
        .unwrap_or_else(|error| panic!("bad ICGEM float {field:?}: {error}"))
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
