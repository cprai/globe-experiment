# Testing & verification

- **No full test suite, no CI.** Verification is the smoke test + manual
  interaction on native Windows. Do not add a CI gate without owner sign-off.
  There are a few render-free unit tests (`cargo test`): in
  `celestial_sphere.rs` covering the galactic star-frame matrix and the IAU
  lunar rotation (`luna_near_side_faces_terra`), in `satellite.rs`
  covering the TLE-free numerical pipeline
  (`numerical_pipeline_holds_circular_leo`), in `ui/instruments/button.rs`
  covering hold-key press persistence past egui's 0.8 s click timeout
  (`hold_key_fires_past_click_timeout`, CPU-only egui passes, no satkit), in
  `ui/py.rs` covering the Python->Rust panel conversion over every registered
  instrument class (+ the non-instrument TypeError path), and in
  `scenes/manual_control_py.rs` the full Python-scene round trip
  (`python_scene_round_trip`: loads the real `scenes/manual_control_py.py`,
  asserts the three converted panels, and click-probes a CPU-only egui pass
  until the script's Run-toggle callback records a `paused` request through
  the property setter and the next `tick_scene` folds it into the
  wrapper-owned clock (the snapshot/request mirror of the `SceneClock` API) —
  main-bin harness only), and in `scenes/solar_system_py.rs` the second
  script's load + Camera Target panel shape
  (`solar_system_script_builds_selector`,
  no satkit — the panel path only reads the clock mirror and the requested
  body index) — run
  them after touching those modules. The two scene tests read the real
  `scenes/*.py` at runtime, so they also stand in for the edit-without-
  rebuild check. Tests touching Python must call
  `engine::py::init()` **before** any `Python::attach` (no auto-initialize;
  the `Once` makes repeated calls safe). Any test needing satkit globals must seed via the `Once`-guarded
  `celestial_sphere::init_satkit_for_tests` (the ephemeris seed is set-once
  per process; a second bare `init_satkit` panics in the shared test binary).
  `cargo test` builds and runs the shared-engine tests **twice** — once per
  binary's test harness (both bin roots compile the shared `engine`); each
  harness is its own process, so the `Once` seeding still holds.
- **`cargo clippy`** — run heavily, aim warning-free. Does not validate WGSL.
- **After every shader edit**: `naga --compact --capabilities none
  shaders/scene.wgsl`. This is the same naga the app links through wgpu —
  authoritative. No output file = validate only. `Validation successful` +
  exit 0 = good. Keep the naga CLI version aligned with `Cargo.lock`.
  **A clean `cargo build` proves nothing about the shader** (naga compiles it
  at runtime, not during the cargo build).
- **`wgsl-analyzer` is a secondary, spec-strict linter** (LSP only). Its CLI
  subcommands (`parse`, `diagnostics`, `unresolved-references`) are stubs
  that panic — do not use them. The only working path is the LSP server
  (`wgsl-analyzer` with no subcommand, JSON-RPC over stdio) with pull
  diagnostics (`textDocument/diagnostic`), not push. It is stricter than naga
  and produces false positives (e.g. the `hash3` bit-mix needed extra
  parentheses); treat its errors as worth investigating but confirm with an
  actual run. Naga is authoritative.
- **Manual pass after risky changes**: pan, flick (inertia), zoom to
  min/max, tilt to clamp, play/pause + speed slider (watch Sol, stars, and
  satellite advance together), window resize, minimize/restore. Confirm a
  paused scene stays frozen (frames keep rendering — the loop is
  unconditional — but nothing on screen moves), and that a minimized
  (occluded) window stops rendering without pinning a CPU core.
- **After atmosphere-constant or mapping changes**, verify **both** the bake
  and shader sides and re-run — bit-identical output is the goal for neutral
  changes.

## Astronomical correctness verification (Sol + star backdrop)

How to prove the celestial sphere is astronomically right (not just that it
renders), independent of satkit. Used to validate the full IERS-2010 switch.

**Independent oracle.** Implement Meeus (*Astronomical Algorithms* 2nd ed.) in
pure Python — solar position ch. 25 (apparent RA/Dec, ~0.01 deg) + sidereal
time ch. 12 (GMST/GAST). Shares no code or data with satkit/DE440, so agreement
is a real cross-check. Expect ~0.01 deg residual (Meeus's own theory error), no
systematic bias.

**Sol / Earth orientation — use the subsolar point.** Nothing prints it today
(the `headless` summary used to; the line was removed as non-generic), so
derive it from `CelestialSphere::sol_dir` (world frame: +Y north, +Z lon 0):
`subsolar_lat = asin(sol_dir.y)`, `subsolar_lon = atan2(sol_dir.x, sol_dir.z)`
— via a throwaway debug print, reverted after the check. The relationships:
- `subsolar_lat = solar declination` (delta).
- `subsolar_lon = RA_sol - GAST` (reduce to +/-180). This is epoch-clean (GHA
  is a physical ECEF angle), so it validates the Sol ephemeris AND the sidereal
  Terra-rotation phase that also orients the stars. Run a spread of dates
  (equinoxes/solstices + arbitrary + old, to exercise the EOP range); both
  agree to <0.01 deg.

**Star frame — instrument `star_rot_inv`, then revert.** Behind an env guard in
`CelestialSphere::at`, map a world axis `w` to standard equatorial GCRF via
`p.transpose() * (star_rot_inv * w)` (undo the Y-up permutation), then
`RA=atan2(y,x)`, `Dec=asin(z)`. Check at **J2000.0** (`2000-01-01T12:00:00Z`),
where the J2000/GCRF backdrop and the of-date sky coincide (epoch-clean):
- Terra pole `w=(0,1,0)` -> **Dec = 90.000** (pole sits on the celestial pole).
- Prime-meridian/equator point `w=(0,0,1)` -> **RA = GAST** (the star frame's
  absolute rotational pinning equals sidereal time).
- `star_rot_inv * sol_dir` -> Sol's RA/Dec on the backdrop, matches Meeus (Sol
  lands among the correct constellations).

**Precession sanity (what the IERS tables drive).** Away from J2000 the J2000-
framed pole tilts off Dec 90 by general precession ~20"/yr: at 2024 the pole
reads **Dec ~89.86** (0.136 deg over 24.5 yr) and RA quantities carry the
matching ~0.3 deg precession-in-RA offset vs of-date GAST. These offsets are
correct evolution of the of-date sky relative to the fixed ICRF backdrop, NOT
bugs — their magnitude == precession is the confirmation.

## wgpu 27->29 migration notes (reference for a future version bump)

From the phase-2 migration, in case wgpu bumps again:
- `Instance::new` takes `InstanceDescriptor` by value, no `Default` — use
  `new_without_display_handle()`.
- `DeviceDescriptor` gained `experimental_features` field.
- `get_current_texture()` returns the `CurrentSurfaceTexture` **enum**, not
  `Result` — arms: `Success`/`Suboptimal` (carry the frame), `Lost`/`Outdated`/
  `Timeout` (reconfigure), `Occluded`, `Validation` (panic).
- `PipelineLayoutDescriptor` takes `&[Option<&BindGroupLayout>]` +
  `immediate_size: 0` (replaced `push_constant_ranges`).
- `multiview` -> `multiview_mask` on pipeline and render-pass descriptors.
- Color attachments gained `depth_slice: None`.
- `RenderPassDescriptor` gained `multiview_mask: None`.
- Sampler `mipmap_filter` is `MipmapFilterMode`.
- egui 0.34: `Context::run` -> `run_ui` (closure gets `&mut Ui`);
  `is_pointer_over_area` -> `is_pointer_over_egui`;
  `Renderer::new` takes `RendererOptions`.
