---
paths:
  - "src/scenarios/**/*.rs"
  - "src/snapshot.rs"
---

# Scenarios & valid time range

## Adding a scenario

- One module per past scenario under `src/scenarios/`, each with a `run()`.
  Add a module and a `ScenarioName` variant in `main.rs`.
- Each scenario owns its inline TLE `const`s. The `ISS_TLE` literal is
  **deliberately duplicated** across scenarios that need it — do not factor
  into a shared const.
- Each scenario's `run()` must call `simulation::init()` (= `init_satkit`)
  before any satkit use. This seeds the embedded DE440 ephemeris and the real
  EOP table. See `simulation.md`.

## EOP valid time range (load-bearing accuracy constraint)

Every scenario's epoch window **must** fall inside `[1962-01-01, last
EOP-All.csv entry]` (approximately the build date):

- **Below 1962**: EOP doesn't exist; satkit silently returns zeros for all EOP
  lookups. The satellite transform falls back to EOP-free accuracy.
- **Above last entry**: satkit does constant extrapolation, silently degrading
  accuracy. Past-only keeps all scenarios below this by construction.
- **Out-of-range = does not meet the accuracy bar.** Flag it rather than
  shipping a silently-degraded result.

When adding a scenario, validate its `[start, end]` epochs against the
bundled file's first/last MJD (and 1962 as a hard lower bound).

## Render / snapshot mode

**`snapshot` deliberately does NOT enforce the EOP range.** The render mode
(`src/snapshot.rs`) accepts any datetime and degrades silently outside range.
Do not add a range check there — the caller owns the time and the behavior
is documented.
