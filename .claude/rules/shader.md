---
paths:
  - "src/engine/shaders/scene.wgsl"
---

# Shader rules & invariants

One WGSL module, five passes: stars (full-screen quad), the single body
impostor shared by all nine bodies, atmosphere, orbit path, markers. CPU-side
pass setup and the reversed-Z/floating-origin stories live in `renderer.md`
and `camera.md`.

## Output calibration (load-bearing)

- **Surface format is non-sRGB, deliberately** (headless target too). Every
  look-tuning constant in `scene.wgsl` is calibrated to it. Do not switch to
  sRGB.
- **No HDR, no bloom.** LDR only; a real bloom pass is explicitly declined.
  The additive Sol disc + glow falloff in `fs_stars` is the approved
  substitute.

## Invariants

- **Every body is an impostor**: the fragment shader ray-traces the triaxial
  ellipsoid — perspective (eye-ray via `inv_view_proj`) for a near body,
  orthographic (parallel-ray, f32-safe at any distance) for a distant one —
  writes `frag_depth` (reversed-Z: nearer = larger; the only depth-writing
  pass), and shades per the body's `BODY_FLAG_*` bits (bare Lambert up to the
  full Terra look).
- **Terminator / night-side darkening must use the GEOMETRIC normal**
  (`dot(n_geo, sol)`), never the bump-mapped normal — bump detail on the
  day/night edge speckles it.
- **Every position uniform is render-frame (camera-target-local)**; there is
  no `render_origin` uniform and no `sol_dir` — derive Sol directions from
  `sol_pos`.
- **Eclipse shadows are analytic** (`sol_visibility`: soft two-disk overlap),
  generic via each body's uniform occluder list + per-body Sol angular
  radius. No shadow maps.
- `fs_atmosphere` does an explicit ray-sphere **Luna occlusion** check (the
  atmosphere pass does not depth-test, so without it the additive glow bleeds
  over a nearer Luna from a Luna-orbit view).
- The star texture is drawn in **galactic** coordinates: the shader-facing
  matrix folds a static galactic->equatorial offset onto the equatorial
  `star_rot_inv` (see `simulation.md`). Star lookup is camera-relative view
  direction only (see `camera.md`).
- `MAX_OCCLUDERS`, `PLANET_QUAD_MARGIN`, and the `BODY_FLAG_*` bits must
  match `src/engine/renderer/mod.rs`; the atmosphere geometry constants must
  match `build.rs` (see `atmosphere.md`).

## Look-tuning discipline

- Look-tuning constants (file-top `const` block) drift between sessions —
  the source is always authoritative; do not trust remembered values.
- Tune and feel-test on a native Windows release build; WSLg cannot validate
  exact colors or interaction feel.
- Two deliberate, owner-confirmed departures from the original plan (do not
  "fix"): `NIGHT_DARKNESS > 1` makes the unlit hemisphere slightly brighter
  than daylight so Terra reads bright all the way around, and
  `EMISSIVE_THRESHOLD` is deliberately permissive for the city mask.
