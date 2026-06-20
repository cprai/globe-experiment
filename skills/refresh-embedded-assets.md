# Skill: Refresh an embedded/baked asset

All baked/transcoded assets land in `OUT_DIR` and are `include_bytes!`-ed.
`build.rs` short-circuits when an output is already present, so refreshing
one means **deleting the stale output** and rebuilding. (`OUT_DIR` is the
gitignored cargo build dir for the crate.)

## Tools
- `cargo` (the rebuild re-runs `build.rs`), `rm`

## Find OUT_DIR
```sh
# the build-script OUT_DIR for this crate (under target/.../build/<crate>-*/out)
find target -type d -name out -path '*globe-experiment*' 2>/dev/null
```

## Assets and how to refresh each
- **A texture** (`*.ktx2`): the `.ktx2` **is** the cache. Delete the stale
  one to re-download + re-BC7-encode it. (With no on-disk source there's
  nothing to re-encode from, so refreshing a texture or changing encoder
  settings *requires* deleting the stale `.ktx2`.)
  ```sh
  rm "$OUT_DIR"/<texture>.ktx2 && cargo build --release
  ```
- **The atmosphere LUTs** (`*.ktx2` f16): delete them to force a rebake from
  `build.rs`'s `mod atmosphere` (do this after `edit-atmosphere-constants`).
- **EOP snapshot** (`EOP-All.csv`): delete it to pull a fresher CelesTrak
  snapshot. This also moves the scenario upper-bound (last data row).
  ```sh
  rm "$OUT_DIR"/EOP-All.csv && cargo build --release   # needs network
  ```
- **JPL ephemeris** (`linux_p1550p2650.440`, ~98 MB): delete to re-download
  verbatim (rarely needed).

## Notes
- Re-downloads need a **network connection** (first/refreshing build only).
- `cargo::rerun-if-changed` points at the `OUT_DIR` copy, so a present
  output is never re-downloaded — deletion is the trigger.
- No `assets/` directory is created; the source images never hit disk (only
  the `.ktx2` cache and the two verbatim embeds are stored).
