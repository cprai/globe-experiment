---
paths:
  - "crates/engine/build.rs"
  - "crates/engine/src/shaders/scene.wgsl"
---

# Atmosphere model — constants & sync rules

## Constants that MUST stay in sync (no compile/runtime check)

Atmosphere medium + geometry constants exist in THREE places: `build.rs mod
atmosphere` (LUT bake), `crates/engine/src/shaders/scene.wgsl` (geometric twins + `MIE_G`), and
`renderer::ATMOSPHERE_TOP_KM` (sizes the atmosphere quad). The inscatter LUT
parameterization (split row mapping, reference-point choice, Bruneton
transmittance mapping) is independently implemented in both `build.rs` and
`scene.wgsl`. Any mismatch silently corrupts the atmosphere — change one,
change the others, and verify bake + shader stay bit-identical for neutral
changes.

The LUT bake runs unconditionally (sub-second), so the CPU-side tables never
go stale; the WGSL twins still need manual sync.

## Medium constants (per-km coefficients, altitude h in km)

- **Rayleigh**: sigma_s_R(h) = (5.802, 13.558, 33.1)e-3 * exp(-h/8)
  (scattering = extinction; no absorption)
- **Mie**: sigma_s_M(h) = 3.996e-3 * exp(-h/1.2), extinction 4.40e-3 *
  exp(-h/1.2), phase asymmetry g = 0.8
- **Ozone**: absorption (0.650, 1.881, 0.085)e-3 * max(0, 1 - |h-25|/15)
- **Geometry**: PLANET_RADIUS_KM 6360, ATMOSPHERE_TOP_KM 6460 (a sphere, not
  the WGS84 ellipsoid — the parameterization requires spherical symmetry)

## LUT parameterization

**Transmittance (256x64)** — Bruneton (r, mu), resolution concentrated near
the horizon:
- y: `x_r = sqrt(r^2 - Rp^2) / H_top`, `H_top = sqrt(Ra^2 - Rp^2)`
- x: `x_mu = (d - d_min)/(d_max - d_min)`,
  `d = -r*mu + sqrt(r^2*(mu^2-1) + Ra^2)`, `d_min = Ra - r`,
  `d_max = rho + H_top`
- Returns 0 below the geometric horizon cosine. `fs_planet`'s ATMO_LIT path
  uses `T(Rp + 0.1, cos_sol)` as the color of sunlight at ground.

**Inscatter (2 x 256x128)** — split row mapping over impact parameter b:
- Lower half (ground-hitting, b < Rp): `v = 0.5 * clamp(b/Rp)`
- Upper half (limb): `v = 0.5 + 0.5 * clamp((b-Rp)/(Ra-Rp))`
- x: `u = mu_ref * 0.5 + 0.5`, mu_ref = Sol cosine at the reference point
  (ground hit for ground rays, closest approach for limb rays).
- Phase functions factor out (`dir·sun` is constant along a straight ray):
  `L = Phi_R * Sigma_R + Phi_M * Sigma_M`; the LUT stores Sigma without
  phase.

## f16 gotcha

f16 max is 65504, min normal ~6e-5. Keep large scale factors (e.g.
`SOL_INTENSITY`) **in the shader, not the bake**.
