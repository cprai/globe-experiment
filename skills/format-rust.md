# Skill: Format Rust code

Format all `.rs` files. rustfmt (with the checked-in `rustfmt.toml`) is the
**sole** formatting authority — don't hand-format, and keep diffs limited to
real changes.

## Tools
- `cargo +nightly fmt` (nightly toolchain required)

## Command
```sh
cargo +nightly fmt
```
Check-only (no writes):
```sh
cargo +nightly fmt -- --check
```

## Why nightly
- `rustfmt.toml` enables `wrap_comments`, which is an **unstable** option.
  Plain stable `cargo fmt` ignores it (emitting a warning) and leaves
  comments unwrapped. So formatting **must** run on nightly. The nightly
  requirement is formatting-only — the build/run toolchain stays stable.
- Always reflow comments and let `wrap_comments` do the wrapping — never
  hand-wrap `//` / `///` / `//!` text.

## Watch reflow on math (important)
`wrap_comments` re-wraps purely on width and is blind to meaning, so it can
insert a line break in the **middle of a mathematical formula** (e.g.
splitting `cos_sun = dot(n_geo, sun)`). After every reflow, scan the diff
for any line break that lands inside a formula/equation and fix it —
reword the surrounding comment so the formula stays on one line. Treat a
formula split across a wrap as a bug to fix, not accept.

## Scope
- Runs on `.rs` only. It does **not** touch `shaders/globe.wgsl` — use
  `format-wgsl` for that.
- Keep all source ASCII-only (golden rule); rustfmt won't fix non-ASCII.
