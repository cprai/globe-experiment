---
paths:
  - "build.rs"
  - "shaders/globe.wgsl"
---

# Atmosphere model — constants & sync rules

## Constants that MUST stay in sync

**Atmosphere medium + geometry constants exist in TWO places:**
1. `build.rs mod atmosphere` (LUT bake, CPU side)
2. `shaders/globe.wgsl` (geometric twins + `MIE_G`)

Change one, change the other. A mismatch silently corrupts the atmosphere —
there is no compile-time or runtime check.

**Inscatter LUT parameterization** (split row mapping, reference-point choice,
Bruneton transmittance mapping) is independently implemented in both `build.rs`
and `globe.wgsl`. A mismatch silently corrupts the atmosphere.

**The LUT bake runs unconditionally** (sub-second), so the CPU-side tables
can never go stale after a constants tweak. The WGSL twins still need manual
sync.

## Medium constants (build.rs mod atmosphere — per-km coefficients)

At altitude h (km):
- **Rayleigh**: sigma_s_R(h) = (5.802, 13.558, 33.1)e-3 * exp(-h/8)
  (scattering = extinction; no absorption)
- **Mie**: sigma_s_M(h) = 3.996e-3 * exp(-h/1.2),
  extinction 4.40e-3 * exp(-h/1.2), phase asymmetry g = 0.8
- **Ozone**: absorption (0.650, 1.881, 0.085)e-3 * max(0, 1 - |h-25|/15)
  (tent peak 25 km, +/- 15 km)
- **Geometry**: PLANET_RADIUS_KM 6360, ATMOSPHERE_TOP_KM 6460

## LUT parameterization

### Transmittance LUT (256x64)

Bruneton (r, mu) parameterization — resolution concentrates near the horizon:
- y axis: `x_r = sqrt(r^2 - Rp^2) / H_top` where `H_top = sqrt(Ra^2 - Rp^2)`
- x axis: `x_mu = (d - d_min)/(d_max - d_min)`,
  `d = -r*mu + sqrt(r^2*(mu^2-1) + Ra^2)`, `d_min = Ra - r`,
  `d_max = rho + H_top`
- Returns 0 when mu is below the geometric horizon cosine (planet shadow).
- `fs_main` uses `T(Rp + 0.1, cos_sun)` as the color of sunlight at ground.

### Inscatter LUTs (2 x 256x128)

Split row mapping (implemented identically in bake and fs_atmosphere):
- Lower half (ground-hitting rays, b < Rp): `v = 0.5 * clamp(b/Rp)`
- Upper half (limb rays): `v = 0.5 + 0.5 * clamp((b-Rp)/(Ra-Rp))`
- x axis: `u = mu_ref * 0.5 + 0.5` where mu_ref = sun cosine at reference
  point (ground hit for ground rays, closest approach for limb rays).
- Phase functions factor out: `L = Phi_R * Sigma_R + Phi_M * Sigma_M`;
  LUT stores Sigma without phase.

## Gotchas when modifying

- Any change to medium constants, split mapping, reference-point choice, or
  Bruneton mapping must be made in **both** `build.rs mod atmosphere` and
  `globe.wgsl`.
- f16 max is 65504, min normal ~6e-5. Keep large scale factors (e.g.
  `SUN_INTENSITY`) **in the shader, not the bake**.
- After any atmosphere change, re-run and verify **both** bake and shader
  produce bit-identical output for neutral changes.
