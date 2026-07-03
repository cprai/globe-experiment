# Backlog / open follow-ups

**None is scheduled work or a bug. Confirm with owner before starting.**

## Re-adding GPU texture compression (BC7 + ASTC)

Phase 14 removed compression for portability. GPU memory cost: ~670 MB
uncompressed vs ~165 MB BC7. **Not a simple revert** — a plain BC7 revert
breaks Apple Silicon (Metal: ASTC/ETC2 only, not BC/S3TC).

**Full multiplatform re-add requires both formats:**
- Produce a BC7 *and* an ASTC KTX2 per texture at build time.
- Select at runtime from adapter caps (drop the unconditional
  `Features::TEXTURE_COMPRESSION_BC`; query for BC *or* ASTC; panic with a
  clear message if neither present).
- `intel_tex_2` does not do ASTC — a separate ASTC encoder is needed.
- `cfg`-gate embedded bytes where the target GPU is unambiguous
  (`aarch64-apple-darwin` → ASTC only; x86_64 → BC7); embed **both** for
  aarch64 Linux (heterogeneous: discrete NVIDIA/AMD has BC, ARM SoC has ASTC).
- Build-host problem: `intel_tex_2` ships prebuilt x86_64 ISPC objects,
  failing on native aarch64 build hosts. Fix: download pre-baked KTX2 at
  build time (recommended), or swap to a portable pure-Rust BC7+ASTC encoder.
- **Cheaper partial win**: downsize all textures to 4K (quarters VRAM, no
  feature requirement). Current zoom rarely samples past 4K density.

## Other open items

- **Downsize textures to 4K** in `build.rs` — quarter decode/upload and VRAM;
  ~134 MB each uncompressed.
- **Expose emissive params** (threshold/strength/color/fade) as egui controls
  for interactive tuning.
- **No heading control, no fly-to animation, no tile streaming.**
- **Second noise octave** in `value_noise_3d` if grain reads too regular
  (keep fixed-scale to preserve coherent wipe).
- **No satellite markers in render mode** (deliberate). If wanted: implement
  a headless `Simulation` variant (or a helper building a clock + celestial
  sphere at an instant) feeding a scenario's `frame_state`, instead of the
  current direct `RenderState` construction in `snapshot::run`.
- **Real bloom post-process** — explicitly **declined** for now.
