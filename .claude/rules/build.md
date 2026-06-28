---
paths:
  - "build.rs"
  - "Cargo.toml"
---

# Build pipeline rules

## Assets are in OUT_DIR — no assets/ dir

Everything the runtime `include_bytes!`-es lands in `OUT_DIR`. There is no
`assets/` directory. `cargo::rerun-if-changed=OUT_DIR/<name>` is emitted per
file so deleting one triggers a re-download or re-bake on the next build.

## EMBEDS table — verbatim downloads

`embed_verbatim(embed, out_dir)` downloads to `OUT_DIR` unless already
present. Currently eleven entries:
- **JPL DE440 ephemeris** `linux_p1550p2650.440` (~98 MiB) — embedded into
  the binary; loaded via `jplephem::init_from_bytes`.
- **CelesTrak `EOP-All.csv`** (~2-3 MiB) — embedded; loaded via
  `earth_orientation_params::init_from_bytes`.
- **IERS 2010 tables** `tab5.2a.txt`, `tab5.2b.txt`, `tab5.2d.txt` (KB each,
  from satkit's astrokit-astro-data bucket) — embedded; loaded via
  `frametransform::init_iers_table_from_bytes` for the full celestial-sphere
  GCRF<->ITRF transforms.
- **Six 8K Earth/star/Moon textures** (JPEG/TIFF verbatim): `8k_earth_daymap.jpg`,
  `8k_earth_nightmap.jpg`, `8k_earth_normal_map.tif`, `8k_earth_specular_map.tif`,
  `8k_stars_milky_way.jpg`, `8k_moon.jpg` (lunar albedo). Decoded at **runtime**
  by `renderer::upload_image` (not at build time). Whether each is sRGB or
  linear is decided in the renderer, not here.

## Atmosphere LUT bake

`bake_luts` runs the inline `atmosphere::bake()` and writes three f16 KTX2
tables (transmittance 256x64, two inscatter 256x128). Runs
**unconditionally** — the tables can never go stale after a constants tweak.
The LUTs are the **only** baked/KTX2 artifacts.

## No build-side image decode or compression

`build.rs` does not use the `image` crate. Textures are downloaded verbatim
and decoded at runtime. `intel_tex_2` and BC7 compression are removed
(phase 14, for multiplatform support). Do not re-add build-side decode or
compression without reading `backlog.md` first.

## C toolchain is required

`ring` (pulled by `ureq`, used for HTTPS downloads in `build.rs`) compiles
C/asm through `cc`. This is portable across all six targets but not pure-Rust.
No practical workaround (`aws-lc-rs` also requires C).

## Profile overrides

`[profile.dev.package.*] opt-level = 3` for `image`, `zune-jpeg`, `zune-core`,
`tiff`, `miniz_oxide`, `weezl` — speeds up the runtime texture decode (five
33 MP images at startup) in dev builds. These are in `Cargo.toml`, not
`build.rs`.
