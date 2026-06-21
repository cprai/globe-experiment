# Multiplatform Refactor Plan

Goal: support **Windows, Linux, and macOS**, each on **x86_64 and aarch64**
(6 target combinations). This document inventories everything in the current
code that is platform- or architecture-specific, ranks the risk, and proposes
the change for each. Nothing here is implemented yet -- it is a plan plus a set
of open questions for the owner.

The headline issue is texture compression: it splits into **two independent
axes** that are easy to conflate, so they are called out separately below.

- **Runtime axis** -- the *target GPU* must support the compressed format we
  upload. This is decided by the machine that *runs* the app.
- **Build axis** -- the BC7 encoder (`intel_tex_2`) runs at build time on the
  *build host*. This is decided by the machine that *compiles* the app, and is
  independent of the runtime target (the KTX2 output is just bytes).

## Decisions (locked by the owner)

1. **Apple Silicon (macOS aarch64) is in scope** -- implement a full **ASTC**
   texture path with runtime/compile-time format selection (option 1B below).
2. **Native aarch64 builds are required** -- the build must work when compiled
   *on* an aarch64 host (ARM Linux, Apple Silicon), not only cross-compiled.
3. **Users build from source** -- there is no prebuilt-binary shield; the
   build-host issues (`intel_tex_2`, C++ linkage) reach end users directly and
   must be solved, not documented around.

Combined effect: **every** item below is in scope, and the build can no longer
depend on an x86_64-only encoder. The two practical ways to satisfy "native
aarch64 + build from source" are (i) replace `intel_tex_2` with portable
encoders that run on any host arch, or (ii) stop encoding at build time and
**download pre-baked KTX2 artifacts** (the same pattern `build.rs` already uses
for the ~98 MB ephemeris). See #2 for the trade-off; this is the one remaining
implementation choice.

---

## Current platform-specific surface (inventory)

| # | Location | What it assumes | Affected targets |
|---|----------|-----------------|------------------|
| 1 | `renderer/mod.rs` `request_adapter_device` -> `required_features: Features::TEXTURE_COMPRESSION_BC` | The target GPU supports BC/S3TC | **macOS aarch64 (Apple Silicon) breaks** |
| 2 | `build.rs` (`intel_tex_2` build-dep, ISPC BC7 encoder) | Build host is x86_64 with prebuilt ISPC objects | **Native aarch64 builds (ARM Linux, Apple Silicon) may break** |
| 3 | `.cargo/config.toml` | `-lstdc++` only for `x86_64-unknown-linux-gnu` | aarch64 Linux build host; macOS build host |
| 4 | `build.rs` / `celestial_sphere.rs` ephemeris file `linux_p1550p2650.440` | (name only) -- content is little-endian DE440 | None functionally; misleading name |
| 5 | `.devcontainer/devcontainer.json` | x86_64 + lavapipe dev env (mostly already arch-tolerant) | Dev environment only |

Everything else surveyed is portable: satkit and its deps are pure Rust
(no `*-sys`/`cc`/native deps once the `download` feature is off); winit's
event loop is created and run on the main thread from `main()` (satisfies the
macOS main-thread requirement); the wgpu surface setup (non-sRGB format pick,
`AutoVsync`, no depth buffer, draw-order occlusion) is backend-agnostic and
works over DX12/Vulkan/Metal/GL; the headless path renders to `Rgba8Unorm`
with explicit RGBA byte order (no endianness/channel-order assumption).

---

## 1. CRITICAL -- Apple Silicon (macOS aarch64) cannot use BC7

**Problem.** `request_adapter_device` hard-requires
`wgpu::Features::TEXTURE_COMPRESSION_BC` and `.expect("request device")`s.
Apple Silicon GPUs (M1/M2/M3/...) expose **only ASTC and ETC2** through Metal,
**not** BC/S3TC. So on every Apple Silicon Mac the device request fails and the
app panics at startup. (Intel Macs, which are x86_64 and carry AMD/Intel GPUs,
*do* support BC -- so x86_64 macOS is fine; this is strictly an aarch64-macOS
problem.)

This is the load-bearing blocker for full 6-target coverage, and it touches the
texture pipeline end to end -- not a one-liner. It also collides head-on with a
CLAUDE.md golden rule (`Device requires Features::TEXTURE_COMPRESSION_BC` ...
"Universal on desktop GPUs") and the "BC support is universal on desktop GPUs"
comment in `request_adapter_device`; both assumptions are simply false on
Apple Silicon and must be revised in the same change.

**Decision: ASTC path (1B).** Bake an **ASTC** KTX2 variant of each
color/data texture, select the format from GPU capabilities, and branch
`upload_ktx2`'s format map accordingly. ASTC is GPU-compressed and visually
equivalent. Note the look-tuning calibration is to the *non-sRGB surface*, not
to BC7 specifically, so swapping compression should be visually neutral -- but
it must be eyeballed on Apple Silicon hardware, and ASTC is lossy with a
different error profile than BC7, so compare against the x86_64 reference frame.

**Format selection -- prefer compile-time `cfg` gating over runtime-only.**
The needed format is knowable from the target at compile time, which lets us
embed only one variant per build and avoid doubling the (already ~260 MB)
binary:

- `aarch64-apple-darwin` (Apple Silicon, no BC, no eGPU) -> embed **ASTC only**.
- `x86_64-apple-darwin` (Intel Mac, AMD/Intel GPU) -> embed **BC7** (BC works).
- All Windows/Linux desktop targets with discrete GPUs -> embed **BC7**.
- **Caveat -- aarch64 Linux is heterogeneous:** an ARM box with an NVIDIA/AMD
  discrete GPU supports BC, but ARM SoC GPUs (Mali/Adreno/etc.) support
  ASTC/ETC, not BC. A pure compile-time `cfg(target_arch/os)` cannot tell these
  apart. So keep a **runtime capability check** as the source of truth:
  query adapter features, pick BC vs ASTC, and **panic with a clear message**
  (not the current bare `.expect`) if the embedded variant is unsupported. For
  aarch64 Linux specifically, consider embedding **both** BC7 and ASTC and
  choosing at runtime, accepting the size cost, until the target GPU set is
  pinned down.

So: `cfg`-gate which KTX2 bytes are `include_bytes!`-ed where the answer is
unambiguous (Apple), and drive the actual `wgpu` format + required-feature from
the adapter at runtime everywhere.

**Touch list:** `build.rs` (ASTC bake + KTX2 wrap for ASTC formats; an ASTC
encoder -- `intel_tex_2` does not do ASTC -- see #2 for the encoder strategy),
`renderer/mod.rs` (`request_adapter_device` feature selection by adapter caps
with a descriptive panic; `upload_ktx2` format map gains ASTC formats; the
`texture_inputs` list becomes `cfg`/runtime-selected; the "BC is universal"
comment), `renderer/headless.rs` (shares the device path -- inherits the fix),
`MEMORY.md` + `CLAUDE.md` golden rules + `README.md`.

---

## 2. HIGH -- `intel_tex_2` (BC7 encoder) on a non-x86_64 build host

**Problem.** `intel_tex_2` wraps Intel's ISPC texture compressor and links
**prebuilt x86_64 ISPC object files**. It is a **build-dependency**, so it runs
on the **build host**, compiled for the host target -- *not* the runtime
target. Consequences:

- **Cross-compiling** from an x86_64 host to any aarch64 target keeps the
  encoder on x86_64 -> **fine** (the KTX2 BC7 output is host-independent data).
- **Native builds on an aarch64 host** (ARM Linux box, or `cargo build` on an
  Apple Silicon Mac) run `intel_tex_2` on aarch64, where the prebuilt ISPC
  objects likely **don't exist** -> build failure. (Needs verification against
  the exact `intel_tex_2` 0.5 release; ISPC does have NEON targets, but the
  crate may not ship aarch64 prebuilts.)

**Cross-compile is ruled out** (owner requires native aarch64 + build from
source), so `intel_tex_2` as-is cannot stay on the encode path. Two viable
strategies remain; pick one:

- **(B) Download pre-baked KTX2 at build time** (recommended). `build.rs` stops
  encoding and instead downloads already-encoded **BC7 and ASTC** KTX2 files
  (the same download-into-`OUT_DIR` pattern it already uses for the ~98 MB
  ephemeris and `EOP-All.csv`). The build host arch becomes irrelevant -- no
  encoder runs locally -- so native aarch64 (incl. Apple Silicon) builds "just
  work," and `intel_tex_2` + the `.cargo/config.toml` C++ linkage (#3) are
  **deleted entirely**. Cost: the pre-baked artifacts must be hosted somewhere
  stable (release asset / object store); the first build's network dependency
  grows (already required today). Encoding moves to a one-time CI step on
  x86_64. This cleanly satisfies all three locked decisions.

- **(C) Portable (pure-Rust) encoders.** Replace `intel_tex_2` with encoders
  that compile and run on any host arch, for **both** BC7 and ASTC, so
  `build.rs` keeps encoding locally on aarch64. Keeps the "self-contained,
  encode-at-build-time" design. Cost: must find/verify a portable BC7 encoder
  *and* an ASTC encoder of acceptable quality/speed; the output bytes change vs
  `intel_tex_2`, so textures need re-verification; per-build encode time is paid
  on every clean build (including slow ARM hosts). Higher risk than (B).

**Recommended:** **(B)** -- it removes the encoder from end-user machines
entirely, which is exactly the friction "native aarch64 + build from source"
otherwise creates. It also subsumes the ASTC-encoder need from #1 (CI bakes both
formats) and lets #3 be deleted rather than extended.

---

## 3. HIGH -- `.cargo/config.toml` C++ runtime linkage is x86_64-Linux only

**Problem.** The `-lstdc++` link-arg is gated to
`[target.x86_64-unknown-linux-gnu]` because `intel_tex_2`'s ISPC objects pull in
the GCC C++ exception personality. As long as `intel_tex_2` runs on the build
host, the build-script executable needs this on **every** Linux host arch:

- **aarch64 Linux build host:** add `[target.aarch64-unknown-linux-gnu]` with
  the same `-lstdc++` (only needed if building natively on aarch64 -- moot under
  the cross-compile policy of #2A, or if #2B/#2C removes `intel_tex_2`).
- **macOS build host:** Apple's toolchain links **libc++** (`-lc++`), not
  libstdc++; verify whether `intel_tex_2` resolves its C++ personality there
  automatically (clang default) or needs an explicit `[target.*-apple-darwin]`
  `-lc++`.
- **Windows MSVC:** unaffected (resolves automatically) -- already documented.

**Change:** under the recommended **#2B** (download pre-baked KTX2, no local
encoder), `intel_tex_2` leaves the dependency tree, its ISPC objects are gone,
and **`.cargo/config.toml` can be deleted entirely** -- no C++ runtime to link
on any host. Only if **#2C** (portable encoders) is chosen instead does this
file need extending: add `[target.aarch64-unknown-linux-gnu]` (`-lstdc++`) and
verify/handle the macOS (`-lc++`) case, with the comment updated from
"Linux-only."

---

## 4. LOW -- ephemeris filename `linux_p1550p2650.440` (name only)

The embedded file is JPL's **little-endian** DE440. "Linux" in JPL's path is the
label for the little-endian byte order, **not** an OS dependency. Every target
in scope (Windows/Linux/macOS on x86_64/aarch64) is little-endian, so the
embedded bytes are correct everywhere and **no functional change is needed**.
The only nit is that the name and the surrounding comments read as OS-specific;
worth a one-line clarifying comment so a future reader doesn't think macOS needs
a different file. (If a big-endian target were ever added, this would become
real -- but none is in scope.)

---

## 5. LOW -- dev container is x86_64/lavapipe-flavored (dev env only)

`.devcontainer/devcontainer.json` already lists both x86_64 and aarch64 lavapipe
ICD JSONs in `VK_DRIVER_FILES`, so it is mostly arch-tolerant. It is a
software-rendering Linux dev environment and not a shipping artifact, so it does
not affect end-user platform support. No change required for the goal; revisit
only if we want first-class ARM-Linux dev containers.

---

## Suggested sequencing

1. **Build axis first (#2B):** set up the CI step that bakes BC7 **and** ASTC
   KTX2 for all five textures on x86_64, host the artifacts, and convert
   `build.rs` to download them instead of encoding. Drop `intel_tex_2` and
   delete `.cargo/config.toml` (#3). This unblocks native aarch64 builds
   immediately and provides the ASTC bytes #1 needs.
2. **Runtime axis (#1B):** add ASTC formats to `upload_ktx2`, make
   `request_adapter_device` choose the required feature (BC vs ASTC) from
   adapter caps with a descriptive panic, and `cfg`/runtime-select which KTX2
   gets embedded (ASTC-only on `aarch64-apple-darwin`; runtime-checked on
   aarch64 Linux).
3. **Ephemeris-name note (#4):** add the one-line "little-endian, not OS"
   clarifying comment.
4. **Doc sync:** revise the BC "universal on desktop GPUs" golden rule and the
   `request_adapter_device` comment, plus `README.md`/`MEMORY.md`, in the same
   changes as the code (repo "docs current with code" rule).
5. **Verify per target:** clean smoke-test run (pipelines/bindings valid) plus a
   headless `render` frame compared against the x86_64 BC7 reference image.
   Apple Silicon (ASTC) is the priority hardware check, since its compression
   path is entirely new and its error profile differs from BC7.

---

## Resolved (owner decisions, 2026-06-21)

The three scoping questions are answered and folded into the plan above:

1. **Apple Silicon: in scope** -> full ASTC path (#1B), with `cfg`/runtime
   format selection.
2. **Build host: native aarch64 required** -> `intel_tex_2`'s x86_64-only
   encoder cannot stay; go with #2B (download pre-baked BC7+ASTC KTX2).
3. **Distribution: users build from source** -> build-host friction must be
   removed, not documented around; #2B does exactly that and lets #3
   (`.cargo/config.toml`) be deleted.

No open questions remain; the only implementation fork left is #2B (recommended)
vs #2C (portable pure-Rust encoders), and the aarch64-Linux BC-vs-ASTC GPU
caveat noted in #1, which a runtime capability check covers.
