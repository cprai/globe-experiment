# Skill: Format WGSL

Format `shaders/globe.wgsl`. `wgslfmt` is the formatting authority for
`.wgsl`, just as rustfmt is for `.rs`. Don't hand-format WGSL.

## Tools
- `wgslfmt`

## Command
```sh
wgslfmt shaders/globe.wgsl
```
Check-only (verify without writing):
```sh
wgslfmt --check shaders/globe.wgsl
```

## Notes
- Only touches whitespace/layout, **never tokens**, so it is safe to run on
  the look-tuning `const` block at the top of the shader.
- Keeps output **ASCII-only** (the golden rule still holds).
- It does **not** wrap comments, so the math-reflow caution that applies to
  Rust does not apply here; WGSL comment wrapping is still manual.
- After formatting, run `validate-wgsl-naga` — `wgslfmt` only formats, it
  does not validate the shader.
