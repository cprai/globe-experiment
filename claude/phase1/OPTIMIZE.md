# Startup Performance Ideas

The window opens, then shows a blank widget until the first frame
completes. Everything heavy runs inside `Pipeline::new`
(`src/globe/pipeline.rs`), which iced calls lazily on the first draw:

- Decoding five 33 MP textures (~170 MB compressed → ~670 MB RGBA).
- Uploading all of that to the GPU.
- Baking the atmosphere LUTs (`src/globe/atmosphere.rs`).
- Compiling the WGSL module.

All of it is serial and blocks the first frame. Ideas below, roughly in
bang-for-buck order. Measure first: replace the `dbg!` markers in
`Pipeline::new` with `std::time::Instant` timings so each fix can be
attributed.

## 1. Parallelize the startup work

The five texture decodes and the LUT bake are fully independent — run
them on threads (`std::thread::scope`, or rayon) and the cost drops from
the *sum* of six tasks to the *max* of them. On a typical 8-core machine
that's roughly a 3-4x cut in blank-screen time for ~30 lines of
restructuring. The GPU uploads still happen on the calling thread, but
they're a small fraction of the cost.

- Effort: small.
- Risk: low; the tasks share nothing.

## 2. Move the atmosphere bake to build time

The LUTs are pure functions of the constants in
`src/globe/atmosphere.rs`. `build.rs` already exists — generate the
tables there, write them to `OUT_DIR`, and `include_bytes!` them, making
the runtime bake cost zero.

- Trade-off: the atmosphere constants move to build-script-land, and
  each tweak costs a build-script rerun instead of just an app restart.
  A disk cache keyed on a hash of the constants is the middle ground:
  runtime bake on miss, instant load on hit.
- Effort: medium.
- **(Implemented in phase 2, 2026-06-12 — see the status section of
  claude/phase2/PHASE2_PLAN.md. No hash cache needed: cargo's
  rerun-if-changed on atmosphere.rs is the cache key, and the
  sub-second bake just reruns whenever the script does.)**
- **(Update 2026-06-13: the bake code was inlined directly into
  `build.rs` as `mod atmosphere` and `src/globe/atmosphere.rs` deleted.
  The cache key is now `build.rs` itself — cargo always reruns the
  script when it changes — rather than a separate rerun-if-changed on
  atmosphere.rs.)**

## 3. Shrink or pre-process the textures

Two levels:

- **Downsize**: 4K (4096x2048) instead of 8K quarters the decode and
  upload work. At current zoom levels the renderer rarely samples past
  4K density, so the visual cost is small. Could be done in `build.rs`
  right after download.
- **GPU-compressed formats**: transcode at build time to BC7 (KTX2
  container). No JPEG/TIFF decode at runtime at all — the bytes load
  straight into the texture — plus 4-8x smaller GPU uploads and VRAM
  footprint. This is the "real engine" solution and also what mipmaps
  would want to ride along with.
  **(Implemented in phase 2, 2026-06-12 — see the status section of
  claude/phase2/PHASE2_PLAN.md. Was unlocked by the iced removal:
  iced's `Features::empty()` device had blocked BC formats.)**

- Effort: small (downsize) to large (BC7 pipeline).

## 4. Decouple the window from the heavy load

Even with everything faster, doing decode + upload in the first frame is
architecturally why the widget is blank. The iced-native fix:

1. Embed a tiny placeholder texture set (e.g. 256x128 versions, a few
   KB) so `Pipeline::new` is instant and the first frame renders
   immediately.
2. Decode the full-resolution textures in an async `Task` (thread pool)
   started from the app.
3. When done, send the decoded images through a message, hand them to
   the `Primitive`, and swap the bind group on the next `prepare`.

The globe appears instantly and sharpens a moment later. Fixes the
*perception* of slowness regardless of how fast the load gets.

- Effort: the largest of these; touches app state, messages, and the
  pipeline's texture ownership.

## 5. Smaller wins

- **Load assets from disk at runtime** instead of `include_bytes!`: the
  binary currently embeds ~21 MB, which costs link time on every build
  and a little process-load time. Runtime loading also unblocks
  hot-swapping textures without recompiling.
- **Window-open latency** (before any project code runs) is mostly wgpu
  adapter/device init and iced setup — largely not ours to fix, but
  worth re-measuring after the above to see what floor remains.

## Suggested order

1. Add timings to `Pipeline::new` (attribution).
2. Idea 1 (parallelize) — small effort, big cut.
3. Idea 3a (downsize to 4K in `build.rs`) if the visuals hold up.
4. Idea 2 or 4 depending on whether tweak-iteration speed (2) or
   perceived startup (4) matters more.
