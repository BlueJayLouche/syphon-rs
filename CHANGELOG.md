# Changelog

Versions match the crates published on
[crates.io](https://crates.io/crates/syphon-core).

## Unreleased

Documentation and `syphon-examples` only — no published crate changed, so
nothing here needs a version bump.

Downstream crates were hitting a launch-time failure that looked like a
packaging bug in `syphon-core`:

```
dyld[…]: Library not loaded: @rpath/Syphon.framework/Versions/A/Syphon
  Reason: … (have 'x86_64', need 'arm64')
```

The framework is bundled and universal, but `cargo:rustc-link-arg` applies
only to the emitting package's own targets — so the `-rpath` `syphon-core`
adds for its tests and examples never reaches a dependent's binary. Those
binaries were left with `/Library/Frameworks` as their only Syphon search
path, which breaks as soon as the copy there is a pre-Apple-Silicon,
x86_64-only build left by an old installer. `dyld` resolves `@rpath` against
the first *match*, not the first *loadable* match, so the stale copy wins.

- **README**: new "Linking from your own crate" section with the `build.rs`
  recipe using `DEP_SYPHON_FRAMEWORK_DIR`, which `links = "Syphon"` already
  exported but nothing documented. Covers why the dependency is needed even
  for crates with no Syphon code (Cargo sets `DEP_*` for direct dependents
  only), and how to bundle the framework into a `.app` when consuming from
  crates.io. Troubleshooting gains an entry for the error above; the old
  entry recommending `sudo cp -R … /Library/Frameworks/` is gone, since that
  is what creates the stale copy in the first place.
- **syphon-examples**: `build.rs` now takes the framework from
  `DEP_SYPHON_FRAMEWORK_DIR` and orders it ahead of `/Library/Frameworks`,
  replacing a hardcoded `../syphon-lib` path that only worked inside a
  workspace checkout. Also drops its redundant `rustc-link-lib` lines —
  those propagate from `syphon-core` normally, unlike link args. It now
  serves as the reference implementation the README points to.

## syphon-wgpu 0.4.3, syphon-metal 0.4.2 (2026-09-08)

Fixes a performance defect in 0.4.2's receive fences. **Supersedes 0.4.2 —
use this instead if you set `SYPHON_WGPU_SYNC=event`.**

`SyphonWgpuInput::new` armed `enable_strict_event_sync` while probing the
queue. That call is irreversible and queue-wide, and the input is
constructed when a Syphon *layer* is created — long before it connects to
anything, and whether or not it ever does. So every submit in the host
application paid an extra command buffer for the process lifetime in
exchange for nothing. Profiling a real 8-layer VJ set measured **-1.8 fps
at identical CPU** with a Syphon layer that never connected; after the fix
the same comparison is -0.2 fps, inside run-to-run noise.

- **syphon-metal**: new `wgpu_interop::queue_is_metal` — a capability check
  with no side effects. Additive.
- **syphon-wgpu**: no API change. Strict mode is now armed by
  `queue_wait_for_event`, i.e. on the first blit that actually fences a
  frame. With a live publisher the event path measures a net win:
  24.3% CPU / 36.1 fps to 23.6% / 36.5.
- **syphon-examples**: `wgpu_sender` spends its frame wait in
  `CFRunLoopRunInMode`. Syphon announces servers over distributed
  notifications, so without a run loop it published to nothing — 0 clients
  while a client reported `ServerNotFound`.

## syphon-wgpu 0.4.2, syphon-metal 0.4.1 (2026-09-08)

Cross-queue ordering on the **receive** path now uses `MTLSharedEvent`
instead of blocking the CPU. Opt in with `SYPHON_WGPU_SYNC=event`; the
bounded CPU drain stays the default until this is verified against an
external publisher on hardware.

A Syphon blit runs on its own `MTLCommandQueue`, so Metal's in-queue
ordering never reaches wgpu, and the output texture had two unordered
hazards — not one. The next blit overwriting a texture wgpu was still
sampling was covered, by draining wgpu every frame. wgpu sampling before
the blit landed was covered by nothing. Two shared events close both at
no CPU cost.

- **syphon-metal**: new `wgpu_interop::{SharedEvent, new_shared_event,
  queue_wait_for_event, queue_signal_event, queue_take_pending_signal}`
  (behind the `wgpu` feature). Additive; nothing existing changed.
- **syphon-wgpu**: no API change. `SyphonWgpuInput` fences its blits when
  the environment opts in.

The **publish** path keeps its bounded drain and cannot be converted with
wgpu-hal 30's API — it needs already-submitted work to signal, and
`add_signal_event` only stages for the next submit. See the notes on
`drain_wgpu_before_blit`.

## 0.3.0 (2026-07-09)

Migrated off the deprecated `metal-rs` (`metal` crate) and, at the public
API boundary, off the unmaintained `objc 0.2` — onto `objc2` /
`objc2-metal` / `objc2-io-surface`, the same bindings wgpu-hal 29 already
uses. This removes the RUSTSEC-2024-0436 (`paste`, unmaintained) advisory
from the dependency tree and turns the wgpu↔Metal pointer-punning bridge
into safe clones/borrows.

### Breaking

- **syphon-metal**: `MetalContext` now holds
  `Retained<ProtocolObject<dyn MTLDevice/MTLCommandQueue>>`;
  `from_raw_device` → `from_device`. `IOSurface` is now
  `CFRetained<objc2_io_surface::IOSurfaceRef>` (the `io-surface` crate is
  gone). `create_texture_from_iosurface` / `blit_to_iosurface` take and
  return objc2-metal types; `blit_to_iosurface` returns the *uncommitted*
  `(command buffer, texture)` pair. `wgpu_interop::extract_metal_device` /
  `with_metal_texture` are now safe functions returning objc2-metal types;
  `get_metal_texture_ptr` and `blit_wgpu_to_iosurface` were removed.
  New: `MetalContext::from_wgpu_device` (behind the `wgpu` feature).
- **syphon-core**: `Frame::metal_texture_ptr` and
  `SyphonServer::publish_metal_texture` /
  `new_with_name_and_device(_and_options)` use `*mut objc2::runtime::AnyObject`
  instead of `*mut objc::runtime::Object`. `Frame::iosurface()` returns
  `&objc2_io_surface::IOSurfaceRef`; `Frame::iosurface_ref()` removed.
  `to_nsstring` / `from_nsstring` are no longer exported.
- **syphon-wgpu**: no API changes beyond the types re-exported from the
  crates above; internal Metal interop is deduplicated into
  `syphon_metal::wgpu_interop`.

## syphon-core 0.2.1 (2026-07-04)

### Fixed

- The crates.io package is now actually linkable. `cargo package` follows
  symlinks, so 0.2.0 shipped the framework payload flattened (no
  `Syphon.framework/` wrapper) and the emitted
  `rustc-link-search=framework=` could never satisfy `-framework Syphon`.
  The build script now reassembles a canonical `Syphon.framework`
  (Versions/A + symlinks) in `OUT_DIR` and links against that, from both
  repo checkouts and the published tarball.
- Added `links = "Syphon"`: direct dependents can read
  `DEP_SYPHON_FRAMEWORK_DIR` to locate the reassembled framework (e.g. to
  bundle it into an `.app` or add a dev-run rpath).
- The package now includes only `frameworks/Versions/A/**` (no more
  symlink-followed duplicate payload).

## 0.2.0 (2026-06-13)

### Breaking

- Removed the `metal_device` module and its public API (`MetalDeviceInfo`,
  `default_device`, `available_devices`, `recommended_high_performance_device`,
  `check_device_compatibility`, `validate_device_match`, `get_device_info`).
  Only the system default-device pointer was ever used; it is now obtained
  directly via `MTLCreateSystemDefaultDevice`. Drop these imports if you used them.

### Changed

- Migrated all crates to the **Rust 2024 edition** (MSRV: Rust 1.85).
- `SyphonServerDirectory::servers()` now returns an empty `Vec` when
  `Syphon.framework` isn't loaded, instead of panicking.

### Fixed

- Corrected stale/uncompilable doc examples and cleared all Clippy lints.
- Consolidated documentation: removed the redundant `QUICKSTART`,
  `TROUBLESHOOTING`, `OPTIMIZATION`, and `DOCUMENTATION_INDEX` files in favour of
  the README plus [`ZERO_COPY_IMPLEMENTATION.md`](./ZERO_COPY_IMPLEMENTATION.md).

## 0.1.0 – 0.1.2 (crates.io, 2026)

- **0.1.2** — bundle `Syphon.framework` v5; ship the Syphon license.
- **0.1.1** — fix all compiler warnings; remove stray test files.
- **0.1.0** — first crates.io release of `syphon-core`, `syphon-metal`, `syphon-wgpu`.

---

## Pre-crates.io history

> These entries use the project's original internal version numbers, which
> predate publishing. Internal `0.5.0` corresponds to the first crates.io
> `0.1.0` release.

### Internal 0.5.0 (2026-05-12)

Bumped all dependencies to their current versions. No public API changes — all
wgpu-hal interop is isolated in `syphon-wgpu/src/metal_interop.rs` as designed.

**wgpu 25 → 29** (breaking internal change to the `wgpu-hal` interop layer):

- `as_hal` no longer takes a closure; it returns `Option<impl Deref<Target = A::Device>>`
  directly. Metal device/texture handles come from `raw_device()` and `raw_handle()`.
- `Queue::as_raw()` was **removed**; the blit path now uses a separate
  `metal::CommandQueue` plus `device.poll(PollType::wait_indefinitely())` for ordering.
- `PollType::Wait` is now a struct variant; use `PollType::wait_indefinitely()`.
- `DeviceDescriptor` gained `experimental_features`; `RenderPassColorAttachment`
  gained `depth_slice`; `RenderPassDescriptor` gained `multiview_mask`.
- `InstanceDescriptor` dropped `Default`; use `new_without_display_handle()`, and
  `Instance::new` takes it by value.

Other crate updates: `metal` 0.31→0.33, `thiserror` 1.0→2.0, `pollster` 0.3→0.4,
`core-foundation` 0.9→0.10, `core-graphics` 0.23→0.25, `io-surface` 0.15→0.16,
`cocoa` 0.25→0.26.

### Internal 0.4.0 (2026-03-18)

Performance and API overhaul (backward-compatible unless noted):

- **Push-based frame delivery** via `newFrameHandler` — `connect_with_channel()` /
  `connect_by_info_with_channel()` wake the consumer on each new frame (bounded
  `mpsc::sync_channel(1)`, coalesced).
- **`PublishStatus`** — `publish()` returns an enum instead of `()`, so silent CPU
  fallbacks are impossible.
- **UUID-based connection** — `connect()` returns `AmbiguousServerName` on name
  collisions; use `connect_by_info()` / `find_by_uuid()`.
- **`SyphonOutputConfig` / `ServerOptions`** — configurable pool size and private servers.
- **wgpu input is now truly zero-copy** — IOSurface → Metal blit on Metal backends
  (CPU fallback retained for others and logged).
- **`requestServerAnnounce` discovery** replaces the old 1.5s polling sleep.
- Autorelease pools internalized; explicit `// SAFETY:` comments on all `unsafe impl`.

### Internal 0.3.0 (2024-03-13)

API cleanup: removed the Y-flip compute shader, the `input_fast`/`input_optimized`
variants, and BGRA↔RGBA conversion (native BGRA8Unorm throughout). Reduced to three
examples: `wgpu_sender`, `metal_client`, `simple_client`.

### Internal 0.2.0 (2024-03-07)

Fixed segfaults / "unknown class" crashes by adding `autoreleasepool` wrappers around
all Objective-C interop. Better framework-not-found and GPU error messages.

### Internal 0.1.0 (2024-03-01)

Initial release: `syphon-core` (`SyphonServer`, `SyphonClient`, `SyphonServerDirectory`),
`syphon-wgpu` (`SyphonWgpuOutput`, zero-copy send), `syphon-metal` (`IOSurfacePool`).
