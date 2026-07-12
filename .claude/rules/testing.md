# Testing & verification

- **No full test suite, no CI** (do not add a CI gate without owner
  sign-off). Verification is the smoke test + manual interaction on native
  Windows.
- A few **render-free unit tests** exist (`cargo test`) in
  `celestial_sphere.rs`, `satellite.rs`, `ui/instruments/button.rs`,
  `ui/py.rs`, and the two `*_py` scene modules (which load the real
  `scenes/*.py`, standing in for the edit-without-rebuild check) — run them
  after touching those modules. Gotchas:
  - Tests touching Python must call `engine::py::init()` **before** any
    `Python::attach` (no auto-initialize; the `Once` makes repeats safe).
  - Tests needing satkit globals must seed via the `Once`-guarded
    `celestial_sphere::init_satkit_for_tests` (the ephemeris seed is
    set-once per process; a second bare `init_satkit` panics).
  - `cargo test` runs the shared-engine tests twice (one harness per bin
    root); each harness is its own process, so the `Once` seeding holds.
- **`cargo clippy`** — run heavily, aim warning-free. It does not validate
  WGSL: after every shader edit run `naga --compact --capabilities none
  shaders/scene.wgsl` (authoritative — a clean `cargo build` proves nothing
  about the shader). Keep the naga CLI version aligned with `Cargo.lock`.
- **`wgsl-analyzer`** is a secondary, spec-strict linter: only the LSP server
  path works (pull diagnostics); its CLI subcommands are stubs that panic.
  Stricter than naga, produces false positives — naga stays authoritative.
- **Manual pass after risky changes**: pan, flick, zoom to min/max, tilt to
  clamp, play/pause + speed slider (Sol, stars, and satellites must advance
  together), resize, minimize/restore. A paused scene stays frozen while
  still rendering; a minimized window stops rendering without pinning a core.

## Astronomical correctness verification (Sol + star backdrop)

To prove the celestial sphere astronomically right independent of satkit:
implement Meeus (solar position ch. 25 + sidereal time ch. 12) in pure Python
as an oracle (shares no code/data with DE440; expect ~0.01 deg residual, no
systematic bias).

- **Subsolar point** (derive from `sol_dir` via a throwaway debug print:
  `lat = asin(sol_dir.y)`, `lon = atan2(sol_dir.x, sol_dir.z)`):
  `subsolar_lat = solar declination`; `subsolar_lon = RA_sol - GAST`
  (epoch-clean — validates the Sol ephemeris AND the sidereal rotation phase
  that also orients the stars). Run a spread of dates.
- **Star frame** (instrument `star_rot_inv`, then revert): map a world axis
  to GCRF via `p.transpose() * (star_rot_inv * w)`, check at **J2000.0**
  where the J2000 backdrop and of-date sky coincide: Terra pole -> Dec 90;
  prime-meridian equator point -> RA = GAST; `star_rot_inv * sol_dir` matches
  Meeus.
- **Precession sanity**: away from J2000 the J2000-framed pole tilts off Dec
  90 by ~20"/yr (Dec ~89.86 at 2024) with a matching ~0.3 deg RA offset —
  correct evolution vs the fixed ICRF backdrop, NOT bugs.
