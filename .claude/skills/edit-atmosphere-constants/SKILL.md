---
name: edit-atmosphere-constants
description: Change atmosphere medium/geometry constants or the inscatter LUT parameterization while keeping the duplicated build.rs bake side and crates/engine/src/shaders/scene.wgsl shader side in sync. Use when touching atmosphere math; a change on one side silently corrupts the atmosphere.
---

# Edit atmosphere constants (keep both sides in sync)

The atmosphere medium + geometry constants and the inscatter LUT
parameterization exist **twice** and must stay identical, or the baked LUTs
and the shader that samples them diverge. This skill is the checklist for
changing them safely.

## Tools
- `cargo` (rebuild triggers the LUT bake in `build.rs`), plus the
  `format-wgsl`, `validate-wgsl-naga`, `build-and-run`, and
  `refresh-embedded-assets` skills

## What is duplicated (change one, change the other)
1. **Atmosphere medium + geometry constants:** in `build.rs`'s inline
   `mod atmosphere` (the LUT bake) **and** in `crates/engine/src/shaders/scene.wgsl` (the
   geometric twins + `MIE_G`).
2. **Inscatter LUT parameterization** — implemented independently twice and
   must match exactly:
   - the **split row mapping** (`v`: lower half = ground-hitting rays, upper
     half = limb rays),
   - the **reference-point choice** (ground hit vs. closest approach),
   - the **Bruneton transmittance mapping**.
   These appear in both `build.rs::bake_inscatter` / `bake_transmittance` /
   `sample_transmittance` and in `scene.wgsl::fs_atmosphere` /
   `sol_transmittance`. A change on one side silently corrupts the atmosphere.

## Steps
1. Edit the constant/mapping on **both** the bake side (`build.rs`) and the
   shader side (`crates/engine/src/shaders/scene.wgsl`).
2. **Force a LUT rebake** if you changed the bake: delete the stale LUT
   outputs so `build.rs` regenerates them (see the `refresh-embedded-assets`
   skill), then rebuild.
3. Run the `format-wgsl` + `validate-wgsl-naga` skills on the shader.
4. Run the `build-and-run` skill and re-check the atmosphere visually.

## Neutral-change verification
If a change is meant to be visually neutral, verify **both** the bake
(`build.rs`) and shader (`scene.wgsl`) sides and re-run — **bit-identical
output is the goal** when the change is meant to be neutral.

## Don't
- Don't try to make the LUT model ellipsoidal — it relies on spherical
  symmetry (`PLANET_RADIUS_KM` 6360 / `ATMOSPHERE_TOP_KM` 6460). The visible
  surface is the WGS84 ellipsoid; the atmosphere is intentionally spherical.
