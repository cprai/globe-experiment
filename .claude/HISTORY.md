# Project phase history

Condensed record of the 14 development phases. Explains *why* architectural
decisions were made. See git log for commit-level detail.

---

**Phase 1** (to 2026-06-11) — The app in **iced 0.14** via
`iced::widget::shader`. All rendering/atmosphere math designed here. iced
created the device with `Features::empty()`, blocking compressed textures.
Textures decoded lazily at runtime (first frame); atmosphere LUTs baked on
CPU at startup.

**Phase 2** (2026-06-11 to 06-13) — **Removed iced entirely**; rebuilt on
winit + raw wgpu + egui. Added: build-time BC7/KTX2 texture transcode,
build-time atmosphere LUT bake, hidden-until-ready window. Heavily iterated
the **smoothed-zoom** controller (the current rate-adaptive glide with velocity
bridging). Later: inlined LUT bake into `build.rs`, deleted `atmosphere.rs`,
parallelized `SceneRenderer::new` with rayon.

**Phase 3** (06-13 to 06-15, +06-17 update) — **Shader-only** rewrite of
`fs_main`. Dropped photographic night map as color; whole Earth is day-mapped
+ darkened by sun geometry, with **procedural city lights** (luminance mask +
fixed-grain 3D-noise dither-dissolve + additive glow). 06-17: added
`EMISSIVE_FADE_END` (lights bleed slightly onto daylit side), converted all
source to **ASCII-only**, removed per-file BC7 transcode cache guard.

**Phase 4** (2026-06-18) — **Physical units.** World space moved from a unit
sphere to the **WGS84 reference ellipsoid in kilometers** to host real-scale
orbital simulation. New `earth` module (WGS84 + dynamics constants +
`surface_position`/`geodetic_normal`). Mesh vertex gained explicit geodetic
normal. Camera, projection, star/atmosphere shells all moved to km. Look-tuning
constants untouched; visually identical.

**Phase 5** (2026-06-18) — **Satellite tracking.** Added `satkit` crate +
`satellite` module. Parses an embedded TLE (ISS), propagates with SGP4 to
a fixed datetime, converts TEME->ITRF->geodetic->world-km. 4th render pass
draws a constant-pixel marker; egui shows datetime + lat/lon/alt.

**Phase 6** (2026-06-18) — **Simulation clock.** `Clock`: sim time starts
at TLE epoch and advances by wall-clock dt x multiplier (1x-100x, play/pause).
Satellite propagated on demand each running frame. Clock starts playing, so
the app renders continuously from launch.

**Phase 7** (2026-06-18) — **Ephemeris-driven Sun & celestial sphere.**
Replaced slider-driven `Sun` with `CelestialSphere`: Sun direction from JPL
DE440 (`jplephem::geocentric_pos`), star map oriented by real GCRF<->ITRF
attitude (`q*_approx`). build.rs downloaded DE440 into `data/` (superseded
in phase 9). Sun lat/lon sliders removed; subsolar point shown read-only.

**Phase 8** (2026-06-18) — **Inertial camera.** Camera rig built in the
celestial frame, rotated into world by
`celestial_to_world = star_rot_inv.transpose()`. Camera holds still relative
to stars while Earth spins beneath it. `Camera.lon/lat` are now an
inertial look direction, not geography.

**Phase 9** (2026-06-19) — **Embedded ephemeris + offline EOP.** DE440
binary downloaded into `OUT_DIR` by build.rs; `include_bytes!`-ed and loaded
via `jplephem::init_from_bytes`. `set_datadir`/`data/` gone. **Also**: fixed
stray `satkit-data` dir — every frame transform reads satkit's global EOP
table on first use, which triggers a lazy `create_dir_all`. Fix: `init_satkit`
pre-seeds the EOP singleton from a header-only CSV + `disable_eop_time_warning`
to consume the one-shot lazy load. (Empty seed superseded in phase 10.)

**Phase 10** (2026-06-19) — **Real EOP bundled.** CelesTrak `EOP-All.csv`
downloaded into `OUT_DIR` and embedded. `qteme2itrf` now applies real polar
motion + UT1-UTC (sub-arcsec). Project reframed as **past-only, astronomically-
accurate** satellite simulation tool. Valid EOP range ~1962 to build date;
past-only keeps every scenario in range permanently.

**Phase 11** (2026-06-19) — **Multiple satellites.** `SimulationState` holds
`Vec<Satellite>`; assembled by the scenario. One instanced draw for all
markers (per-instance position + visibility). UI lists one block per satellite.
`ISS_TLE`/`HST_TLE` consts moved from `satellite.rs` into the scenario file
(element-set agnostic module).

**Phase 12** (2026-06-20) — **No `assets/` dir; everything in `OUT_DIR`.**
Textures downloaded into memory and decoded+BC7-encoded in one pass; `.ktx2`
output is the cache. `download_if_missing` gone; `embed_verbatim` for the
satkit data files.

**Phase 13** (2026-06-20) — **Scenarios module + clap CLI.** New
`src/scenarios/` with one module per past scenario (`run()`). `main.rs`
reduced to a pure clap CLI (`scenario <name>` | `render` subcommands).
`ScenarioName` is a `ValueEnum`; bare `scenario` lists available scenarios.
Added `scenarios::iss` (ISS-only) alongside `scenarios::iss_and_hubble`.

**Phase 14** (2026-06-21) — **Drop GPU texture compression for multiplatform.**
BC7/KTX2 transcode removed from `build.rs`; textures downloaded verbatim
(JPEG/TIFF) and decoded at runtime by `image`. Device requests
`Features::empty()` (no BC). Runs on every backend including Apple Silicon.
Deleted `intel_tex_2`, build-side `image`, `.cargo/config.toml`. Trade-off
(owner-accepted): GPU memory ~4x (~670 MB vs ~165 MB BC7); binary bytes
actually shrank (~21 MB JPEG/TIFF vs ~160 MB BC7). Texture compression
re-add is not simple — see `backlog.md`.

---

## Key architectural decisions

Decisions made during the phases above that need their reasoning preserved.

### Module architecture

**`simulation` has no winit/wgpu/egui dependency, and no `Camera` type.**
This was an explicit design decision: `simulation` deals only in glam/satkit
values and receives a resolved `Vec3`/`Mat4` for the camera. This makes
`frame_state` independently testable and keeps any future input-scheme change
(e.g. touch) entirely local to `application`. Enforced by import discipline.

**Event routing: egui first.** `handle_input` feeds every event to
`egui_state.on_window_event` first. If `response.consumed`, the scene
controller never sees it. This replaces iced's `stack![]` overlay capture and
is what makes panel interaction not pan the scene.

**`simulation.advance()` runs before the UI frame.** This means each frame
applies the *previous* frame's play/pause/speed edits — a one-frame (~16 ms)
delay. Accepted: it lets a single `state_at(now)` call per satellite feed
both the marker and the readout without the two diverging mid-frame.

### Rendering

**No depth buffer.** Draw order (stars → surface → atmosphere → markers) +
convex-sphere assumption handles all occlusion. Never needed for this scene;
adding geometry that breaks the convex assumption would require adding a depth
attachment at the same time.

**Camera-relative view direction for the backdrop.** `vs_stars` computes
`relative = world_pos - camera_pos` (linear in the vertex, exact under
interpolation) and uses it for *both* the star lookup and the sun disc.
This keeps sun and stars locked at any orbit/zoom (the backdrop is at
infinity — no parallax, no zoom dependence). This fixed a specific bug where
the sun disc and star field drifted relative to each other.

**Analytic tangent frame in `fs_main`.** `east = normalize((n_geo.z, 0, -n_geo.x))`
(exact normalized d_position/d_longitude); `north = cross(n_geo, east)`.
No per-vertex tangents stored, no seam artifacts. Only imprecise at the poles
where textures are near-constant anyway.

**Atmosphere shell is front-face-culled with additive blending.** The
atmosphere sphere is rendered with front-face culling (shows the far side of
the shell), which spans the whole silhouette including beyond the limb.
Additive blending (One/One) lets it layer aerial perspective on top of the
surface and glow beyond the planet edge.

**No mipmaps (`mip_level_count 1`).** Causes shimmer at far zoom; accepted
trade-off. Mitigations exist (lower `DITHER_SCALE`, narrow city-light
smoothstep) but have not been applied.

### City lights

**Luminance mask from night map, not color.** The night map is used only for
`dot(night, vec3(0.2126, 0.7152, 0.0722))` — the luminance. Displaying night
map color directly would show JPEG compression artifacts as colored fringing
around city clusters.

**Fixed-grain 3D noise for the dither.** `DITHER_SCALE` is constant and the
noise is surface-anchored (`n_geo * DITHER_SCALE`), so the grain never crawls
or reshuffles under zoom, rotation, or sun motion. A noise frequency ramp
toward the terminator was tried and rejected: it made the city-light band
boil/fizz because the per-point dither value changed frame-to-frame.

**`hash3` integer-lattice hash (not `fract(sin(...))`).** At `n_geo * 400`,
the lattice indices reach the hundreds. f32 `sin()` loses mantissa precision
at large arguments and produces visible banding. The integer approach
(cast i32→u32, three large-prime multiplies, XOR-folds, normalize) is
well-conditioned at any scale.

**Hard `step(fade, dither)` dither (not smoothstep).** Each pixel switches
off exactly when `fade` crosses its own fixed `dither` value, so pixels drop
out in a stable order as the terminator sweeps. This is a coherent wipe with
no fizz. A smoothstep would produce a soft ramp per pixel, blurring the
distinction between lit and dark at the terminator edge.

### Atmosphere model

**Precomputed LUTs instead of per-pixel raymarch.** The Hillaire-2020
two-LUT approach (transmittance + inscatter) replaces a per-fragment numerical
integration that would be too expensive at realtime rates. Two mathematical
properties make precomputation work for this scene: (1) phase functions factor
out because `dir·sun` is constant along a straight ray; (2) the scene has
spherical symmetry — a ray is fully described by its impact parameter `b` and
one sun angle `mu_ref`.

**Atmosphere stays spherical.** The LUT parameterization (Bruneton r/mu
mapping, split row mapping, reference-point choice) assumes a sphere. Making
it ellipsoidal would require a fundamentally different integration. The
6360 km sphere radius is close enough to WGS84's mean radius that the
visual difference is invisible.

**LUT bake runs unconditionally.** The bake is sub-second, so running it
every build means the GPU-side tables can never go stale after a constants
tweak. If the bake were conditional (only re-run when source files changed),
a constants-only edit would leave the LUTs lagging silently.

### Satellite pipeline

**Satellite position is not stored; propagated on demand.** `Satellite`
retains the `TLE` struct (because `sgp4` needs `&mut TLE` — it lazily builds
and caches its propagator inside the TLE on first use). Position is computed
by `state_at(time)` each call and returned, never stored on the struct. This
avoids stale position state as the clock advances.

**Geodetic→world path (not direct ITRF permutation).** `ITRFCoord::from_vector
(&itrf).to_geodetic_rad()` → `earth::surface_position(lat, lon) +
earth::geodetic_normal(lat, lon) * altitude_km`. An alternative was to just
apply the axis permutation P to the ITRF vector directly. The geodetic path
was chosen because it guarantees the marker lands on the *exact same* WGS84
ellipsoid the mesh is built from; the two approaches are mathematically
equivalent but the geodetic one can't drift.

**`*_approx` transforms for the backdrop, not full IERS-2010.** The full
`qgcrf2itrf`/`qitrf2gcrf` transforms additionally require the IERS nutation
tables (Tab5A/5B/5D) to be bundled and seeded in `init_satkit`. Without
seeding those, the transforms `.unwrap()` from the data dir, panic, and
recreate `satkit-data`. The `*_approx` residual (~1 arcsec) only affects the
star/sun backdrop — the satellite uses the full `qteme2itrf` and is already
sub-arcsec. Leaving the backdrop at ~1 arcsec is a deliberate accuracy
trade-off, not an oversight.

### Input

**`WindowEvent` not `DeviceEvent` for `CursorMoved`.** Raw `DeviceEvent`
deltas are scaled and accelerated inconsistently across backends (Windows,
X11, Wayland, macOS). `WindowEvent::CursorMoved` gives consistent
OS-processed coordinates. No `Touch`/pinch path: desktop-only by design.

### Build / assets

**No real bloom.** A real glow halo would require a new render pass and
significant complexity. Explicitly declined. The current additive sun disc +
cubic glow falloff in `fs_stars` is the approved substitute.
