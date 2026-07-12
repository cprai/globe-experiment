# Backlog / open follow-ups

**None is scheduled work or a bug. Confirm with owner before starting.**

## Re-adding GPU texture compression (BC7 + ASTC)

Removed for portability (~670 MB uncompressed vs ~165 MB BC7). **Not a simple
revert** — plain BC7 breaks Apple Silicon (Metal: ASTC/ETC2 only). A full
re-add needs BC7 *and* ASTC KTX2 per texture at build time, runtime selection
from adapter caps, and a portable encoder (`intel_tex_2` does no ASTC and
ships x86_64-only ISPC objects — download pre-baked KTX2 or use a pure-Rust
encoder). Cheaper partial win: downsize all textures to 4K (quarters VRAM, no
feature requirement; current zoom rarely samples past 4K density).

## Other open items

- Downsize textures to 4K in `build.rs`.
- Expose emissive params as egui controls for interactive tuning.
- No heading control, no fly-to animation, no tile streaming.
- Second noise octave in `value_noise_3d` if grain reads too regular (keep
  fixed-scale to preserve the coherent wipe).
- No tracked bodies in the `headless` binary (deliberate). If wanted:
  feed a scene's `frame_state` instead of the current direct `RenderState`
  construction (`scenes` lives in the root crate, which the engine-owned
  headless bin cannot see).
- Real bloom post-process — explicitly **declined**.
