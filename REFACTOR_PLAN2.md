# Refactor Plan 2 -- Drop compressed textures, decode at runtime

Supersedes the texture-compression strategy in `REFACTOR_PLAN.md`. Instead of
solving BC7-vs-ASTC per platform, **remove GPU texture compression entirely**:

- `build.rs` downloads the **original** image files (JPEG/TIFF) verbatim into
  `OUT_DIR` -- no transcode, no `intel_tex_2`.
- The runtime `include_bytes!`-es those originals and decodes them with the
  **`image`** crate into RGBA8, then uploads them as **uncompressed**
  `Rgba8Unorm` / `Rgba8UnormSrgb` textures.

This is the maximum-compatibility option: it needs **no GPU compression
feature at all**, so it runs on every backend and GPU (Apple Silicon, ARM SoCs,
integrated GPUs, lavapipe) without per-platform format selection, and it builds
natively on every host arch because no ISPC/x86 encoder is involved. It dissolves
all four findings of `REFACTOR_PLAN.md` (#1 Apple Silicon BC, #2 `intel_tex_2`
host arch, #3 `.cargo/config.toml` C++ linkage, and most doc churn) in one move.

## Owner decisions (locked)

- **Larger binary and larger GPU memory are accepted.** So: embed the two `.tif`
  normal/specular maps **verbatim** (no build-time PNG re-encode), and upload all
  five textures uncompressed RGBA8. Risks #1 (VRAM ~4x) and #2 (TIFF/binary size)
  below are acknowledged and accepted, not mitigated.
- **No mipmaps.** Keep `mip_level_count: 1` exactly as today; the documented
  far-zoom shimmer is left as-is. Do **not** add mip generation or trilinear
  sampling in this change.
- **Status: plan only.** This document is for review; implementation is not yet
  authorized.

---

## Why this maximizes compatibility

| Old constraint | After this change |
|----------------|-------------------|
| Device requires `Features::TEXTURE_COMPRESSION_BC` | **No required features** (`Features::empty()`) -- BC/ASTC support irrelevant |
| BC7 only on desktop GPUs (breaks Apple Silicon / ARM SoCs) | `Rgba8Unorm`/`Rgba8UnormSrgb` are **universally supported and filterable** |
| `intel_tex_2` ISPC encoder is x86_64-prebuilt (breaks native aarch64 build) | No build-time encoder; `build.rs` only downloads + bakes LUTs |
| `.cargo/config.toml` links `-lstdc++` for ISPC | File **deleted** -- nothing pulls in libstdc++ |
| KTX2/BC7 transcode path | Plain `image::load_from_memory(...).to_rgba8()` upload |

The `Rgba16Float` atmosphere LUTs are **not** compressed textures and are out of
scope here -- they stay exactly as they are (baked by `build.rs`, KTX2-wrapped,
uploaded via the existing path). So the `ktx2` dependency stays, used only for
the three LUTs.

---

## Dependency changes (`Cargo.toml` + `.cargo/config.toml`)

**Remove**
- `intel_tex_2` (build-dep) -- the only consumer of ISPC/libstdc++.
- `image` from **build-dependencies** -- `build.rs` no longer decodes anything
  (it downloads originals verbatim; the LUT bake uses only `half`/`bytemuck`/
  `ktx2`).
- `.cargo/config.toml` entirely -- it exists solely for `intel_tex_2`'s C++
  exception personality. (Verify nothing else links C++ first; nothing does.)

**Change**
- Runtime `image` features: `["png"]` -> **`["png", "jpeg", "tiff"]`**. `png`
  stays for the `render`-mode PNG *output*; `jpeg`/`tiff` are added to *decode*
  the embedded source textures (the three JPEGs and two TIFFs). The
  `[profile.dev.package.*]` `opt-level = 3` overrides for `image`, `zune-jpeg`,
  `zune-core`, `tiff`, `weezl`, etc. become load-bearing at **runtime** now (dev
  builds would otherwise decode 33 MP images unoptimized at every launch) --
  keep them; they already exist.

**Keep**
- `ktx2` (build-dep **and** runtime) -- still used for the f16 atmosphere LUTs.
- `ureq` (build-dep) -- still used to download the originals + ephemeris + EOP.

---

## `build.rs` changes

The five entries in `ASSETS` (currently transcoded to BC7) move to a
**verbatim download**, exactly like the existing `EMBEDS` table (ephemeris,
`EOP-All.csv`). They keep their real extensions in `OUT_DIR`
(`8k_earth_daymap.jpg`, `8k_earth_normal_map.tif`, ...).

- Delete `transcode()`, the `RgbaSurface`/`bc7` usage, and the `intel_tex_2`
  import.
- Delete the `width % 4 == 0 && height % 4 == 0` BC7 assertion -- uncompressed
  textures have no block-dimension requirement.
- Keep the per-asset **`srgb: bool`** flag: it no longer picks a KTX2 format but
  is carried through to runtime (see below) to choose `Rgba8UnormSrgb` vs
  `Rgba8Unorm`. Simplest is to encode it by routing assets through two small
  lists, or keep one `{ url, srgb }` table and persist the flag implicitly by
  file naming/order -- the runtime needs to know it (see "Carrying the sRGB
  flag").
- `bake_luts()`, `write_ktx2()`, and `mod atmosphere` are **unchanged**.
- `embed_verbatim()`/`download()` are reused for the textures; the rerun-cache
  semantics (download once into `OUT_DIR`, `rerun-if-changed` on the cached
  file) carry over unchanged.

Net `build.rs` effect: it becomes "download a list of files; bake the LUTs."
No image decoding happens at build time anymore.

### Carrying the sRGB flag to runtime
The runtime must know which textures are color (sRGB) vs data (linear). Options,
pick one:
- **(a)** Hard-code it at the `include_bytes!` site in `renderer/mod.rs` (the
  file set is fixed and small): a `(label, bytes, srgb)` table. Simplest, no
  build/runtime coupling. **Recommended.**
- **(b)** Have `build.rs` emit a tiny generated manifest into `OUT_DIR`. More
  machinery than warranted for five fixed files.

---

## Runtime changes (`renderer/mod.rs`)

### 1. Drop the compression feature
In `request_adapter_device`, change
`required_features: wgpu::Features::TEXTURE_COMPRESSION_BC` to
`wgpu::Features::empty()`. Update the comment (no more "BC is universal on
desktop GPUs"). This is the single line that unblocks every non-BC GPU.

### 2. Split the upload paths
Today `texture_inputs: [(&str, &[u8]); 8]` mixes 5 image textures and 3 LUTs,
all uploaded by `upload_ktx2`. Split into:

- **Five image textures** -- decode + upload via a new
  `upload_image(device, queue, label, bytes, srgb) -> TextureView`:
  ```text
  let img = image::load_from_memory(bytes)?.to_rgba8();   // RGBA8, sRGB-encoded bytes
  let (w, h) = img.dimensions();
  let format = if srgb { Rgba8UnormSrgb } else { Rgba8Unorm };
  device.create_texture_with_data(
      queue, &descriptor{ size: w x h, mip_level_count: 1, format,
                          usage: TEXTURE_BINDING, .. },
      TextureDataOrder::LayerMajor, img.as_raw());
  ```
  sRGB mapping mirrors the old BC7 split exactly: day / night / stars =
  `Rgba8UnormSrgb`; normal / specular = `Rgba8Unorm`. Because the JPEG/TIFF
  bytes decode to the same sRGB-encoded 8-bit values BC7_SRGB stored, sampling
  through an `*Srgb` view yields the same linearized color as before -- the look
  is preserved (and actually *gains* quality: no BC7 block artifacts).

- **Three LUTs** -- keep `upload_ktx2` unchanged (`Rgba16Float`).

Build all eight `TextureView`s in the existing binding order so the
destructuring (`day_view, night_view, normal_view, specular_view,
transmittance_view, inscatter_rayleigh_view, inscatter_mie_view, stars_view`)
and the bind-group entries are untouched.

### 3. Keep the rayon parallelism (now even more valuable)
`GlobeRenderer::new` already `rayon::join`s shader-module compilation against the
parallel texture uploads. Decoding five 33 MP images is now CPU-heavy, so
**decode in parallel** within that same `par_iter` (each task does
decode-then-upload; `Device`/`Queue` are `Send + Sync`). The LUT entries do a
cheap KTX2 parse on the same pool. This keeps startup latency down.

### 4. `upload_ktx2` slims down
Its `match header.format` no longer needs the two BC7 arms
(`BC7_SRGB_BLOCK`/`BC7_UNORM_BLOCK`); only `R16G16B16A16_SFLOAT` remains. The BC7
texture comments throughout `GlobeRenderer::new` get rewritten.

`renderer/headless.rs` shares `GlobeRenderer` and the device path, so it inherits
all of this automatically (its offscreen `Rgba8Unorm` target is unrelated and
stays).

---

## What deliberately does NOT change

- Atmosphere LUTs (bake, KTX2 wrap, `Rgba16Float` upload, `ktx2` dep).
- Bind-group layout, binding indices, samplers (`Filtering` + linear
  filtering work the same on `Rgba8Unorm*`).
- The **non-sRGB surface/headless target** golden rule -- that is about the
  render *output* framebuffer, a separate thing from these *input* textures.
  Using `Rgba8UnormSrgb` for the color *inputs* is correct and matches the old
  BC7_SRGB inputs; it does not touch the output-format rule.
- `mip_level_count: 1` (no mipmaps) -- unchanged by owner decision; the known
  far-zoom shimmer is neither helped nor worsened, and no mip generation /
  trilinear sampling is added.
- All shader code (`globe.wgsl`) -- the sampled values are identical.

---

## Tradeoffs / risks (flag before implementing)

1. **VRAM goes up substantially (ACCEPTED).** Five 8K (8192x4096) textures
   uncompressed at RGBA8 are ~134 MB **each** (~670 MB total) vs ~33 MB each
   (~165 MB) for BC7 -- roughly **4x more GPU memory**. Owner has accepted this
   as the price of maximum compatibility; no mitigation.

2. **Embedded binary size grows (ACCEPTED).** The two **`.tif`** normal/specular
   maps may be large if stored uncompressed (an 8192x4096 uncompressed TIFF is
   ~100 MB+), so embedding them verbatim could make the binary larger than the
   current BC7 embed. Owner has accepted larger binary size, so they are embedded
   **verbatim** -- no build-time PNG re-encode.

3. **Decode now runs at every startup** (and on every headless `render`), ~5x
   33 MP images. Mitigated by the rayon parallel decode and the `opt-level = 3`
   dev overrides, but it is non-zero added launch latency vs the current
   memcpy-only uploads. Expect tens to low-hundreds of ms on a typical desktop.

4. **This reverses a documented phase-1 decision.** CLAUDE.md records that a
   parallel texture **decode** at startup was removed and decode was moved to
   build time; this plan moves decode back to runtime. The crucial distinction:
   the *reverted* implementation used `thread::scope`; here we use **rayon**
   (the sanctioned parallel primitive, already in `GlobeRenderer::new`). Call
   this out explicitly in the CLAUDE.md/MEMORY.md updates so it does not look
   like an accidental re-introduction of the rejected design.

5. **TIFF decode correctness.** The normal/specular `.tif` files must decode
   cleanly via `image`'s `tiff` feature (handling whatever bit depth/compression
   they use); `to_rgba8()` normalizes to 8-bit RGBA. Verify the decoded output
   matches the old BC7 source (a headless `render` diff against the x86_64
   reference frame catches regressions).

---

## Documentation to update (same change, per repo rule)

- **CLAUDE.md:** the `Features::TEXTURE_COMPRESSION_BC` hard-constraint bullet;
  the "Large binary ... ~160 MB of BC7" note; the `.cargo/config.toml` bullet
  (file deleted); the build.rs "downloads five textures and BC7-encodes them"
  description; the "decode/bake moved to build time" / rejected-`thread::scope`
  note (clarify the rayon runtime-decode distinction); the "Device requires
  TEXTURE_COMPRESSION_BC" line.
- **MEMORY.md:** texture pipeline / file-map sections describing BC7/KTX2 for the
  five textures; the binary-size figures; §16 references if any.
- **README.md:** any "first build downloads + BC7-encodes" / requirements text.
- Code comments in `build.rs` (`ASSETS` doc, `transcode` removal) and
  `renderer/mod.rs` (`GlobeRenderer::new`, `upload_ktx2`,
  `request_adapter_device`).
- **`REFACTOR_PLAN.md`:** mark it superseded for the texture-compression axis by
  this plan (the ASTC/`intel_tex_2` work it proposed is no longer needed).

---

## Suggested sequencing

1. `build.rs`: move the five textures to verbatim download; delete `transcode`/
   `intel_tex_2`/the `image` build-dep; check `.tif` sizes (decide #2 mitigation).
2. `Cargo.toml`: drop `intel_tex_2` + build `image`; add `jpeg`,`tiff` to runtime
   `image`. Delete `.cargo/config.toml`.
3. `renderer/mod.rs`: add `upload_image`; split image vs LUT uploads (rayon);
   drop the BC feature; trim `upload_ktx2`; rewrite comments.
4. Doc sync (above).
5. Verify: `naga` shader check is unaffected; run the smoke test (clean
   ~15-25 s run = pipelines/bindings valid); render a headless frame and diff
   against the x86_64 BC7 reference (expect equal-or-better, modulo gone BC7
   artifacts). If possible, smoke-test on a non-BC GPU (Apple Silicon / ARM) to
   confirm the dropped feature requirement actually lets it start.

---

## Open decisions

None outstanding. Scope is settled: verbatim TIFF embed, uncompressed RGBA8
upload, no mipmaps, larger binary/VRAM accepted. Remaining work is purely the
implementation + doc-sync in "Suggested sequencing" above, to be done when the
owner authorizes (currently plan-only).
