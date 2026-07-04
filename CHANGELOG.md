# Changelog

Versions match the crates published on
[crates.io](https://crates.io/crates/syphon-core).

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
