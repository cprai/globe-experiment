# Testing & verification

- **No test suite, no CI.** Verification is the smoke test + manual
  interaction on native Windows. Do not add a CI gate without owner sign-off.
- **`cargo clippy`** — run heavily, aim warning-free. Does not validate WGSL.
- **After every shader edit**: `naga --compact --capabilities none
  shaders/globe.wgsl`. This is the same naga the app links through wgpu —
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
  min/max, tilt to clamp, play/pause + speed slider (watch Sun, stars, and
  satellite advance together), window resize, minimize/restore. Confirm idle
  (paused) renders **zero** frames.
- **After atmosphere-constant or mapping changes**, verify **both** the bake
  and shader sides and re-run — bit-identical output is the goal for neutral
  changes.

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
