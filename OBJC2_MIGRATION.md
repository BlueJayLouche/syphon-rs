# objc2 migration

**Status: done in 0.3.0** — `syphon-metal` and `syphon-wgpu` are fully on
`objc2` / `objc2-metal` / `objc2-io-surface`; `metal-rs` and the
RUSTSEC-2024-0436 `paste` advisory are out of the tree. See CHANGELOG 0.3.0.

## Remaining (deliberately deferred)

`syphon-core`'s *internals* still use `objc 0.2` + `cocoa` + `block` +
`io-surface` for the Syphon framework bindings (msg_send!, delegate blocks,
frame surfaces). Its *public API boundary* is already objc2-typed
(`*mut AnyObject`, `&objc2_io_surface::IOSurfaceRef`), so migrating the
internals is invisible to downstream crates and can happen whenever those
bindings are next touched. Main work items:

- `msg_send!` call sites → objc2 `msg_send!` (different syntax, typed)
- `block::ConcreteBlock` frame handler → `block2::RcBlock`
- `io_surface::IOSurface` frame storage → `CFRetained<IOSurfaceRef>`
  (objc2-io-surface also provides lock/unlock/base_address, which would
  replace the custom `iosurface_ext` FFI module)
- drop `objc`, `objc_id`, `objc-foundation`, `cocoa`, `block`, `io-surface`,
  `core-foundation`, `core-graphics`; drop the `cargo-clippy` check-cfg and
  `deprecated = "allow"` workspace lint workarounds
