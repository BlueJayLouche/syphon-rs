# objc2-metal migration plan

**Status:** not started (deferred 2026-07-09). Low urgency — see "Why deferred".

Migrate `syphon-metal` and `syphon-wgpu` off the deprecated `metal-rs`
(`metal` crate) and the unmaintained `objc 0.2` onto `objc2` + `objc2-metal`.

## Why

- `metal-rs` is deprecated: "Use of this crate is deprecated… please use
  `objc2` and `objc2-metal` instead" — the maintainers are migrating wgpu
  itself to objc2. `objc 0.2` is likewise unmaintained.
- This is what surfaces the RUSTSEC `paste` (RUSTSEC-2024-0436, unmaintained)
  advisory downstream: `paste ← metal 0.33 ← syphon-metal/syphon-wgpu`.

## Why deferred

It's an *unmaintained* warning, not a vulnerability — `cargo audit` does not
fail on it by default (rustjay CI is green). The change is unsafe GPU FFI in
a published crate whose zero-copy correctness can only be verified with a
live Syphon sender/receiver on a real GPU. Not worth the risk until Syphon is
being touched anyway.

## The one big win

wgpu-hal 29 **already returns objc2-metal types** (`objc2-metal 0.3.2` is
in the tree today via wgpu-hal). The current code pointer-puns those into
metal-rs types; after migration the types match, so the fragile bridge
becomes a safe clone/borrow. Zero new dependencies.

## Scope — both crates declare `metal = "0.33"` directly

`paste` only clears when **both** are migrated.

### `syphon-metal/src/lib.rs`
- `MetalContext { device: metal::Device, queue: metal::CommandQueue }`
  → `Retained<ProtocolObject<dyn MTLDevice>>` / `dyn MTLCommandQueue`.
- `system_default()` → `objc2_metal::MTLCreateSystemDefaultDevice()`.
- `create_texture_from_iosurface` — currently raw `objc` `msg_send!`.
  Replace with `objc2-metal`'s safe `MTLTextureDescriptor` setters +
  `newTextureWithDescriptor_iosurface_plane`. The IOSurface arg: keep the
  `io-surface` crate for the pool (it pulls no metal/paste) and pass its
  `IOSurfaceRef` through, or adopt `objc2-io-surface`.
- `blit_to_iosurface` — `new_command_buffer` / `new_blit_command_encoder` /
  `copy_from_texture` / `end_encoding` all map to safe objc2-metal methods.
- `wgpu_interop` module here **duplicates** `syphon-wgpu/metal_interop.rs` —
  consider collapsing to one during the migration.

### `syphon-wgpu/src/metal_interop.rs`
- `extract_metal_device` — currently `objc_retain` + `Device::from_ptr` on a
  punned pointer → just `raw_device().clone()` (a `Retained` clone). Safe.
- `with_metal_texture` — `raw_handle()` already returns
  `&ProtocolObject<dyn MTLTexture>`; pass it directly, drop the cast.

### `syphon-wgpu/src/input.rs`
- `gpu_blit(metal_queue: &metal::CommandQueue, …)` — retype to objc2-metal;
  `metal::Texture::from_ptr(frame.metal_texture_ptr())` wraps the
  `id<MTLTexture>` from `syphon-core`. objc2 equivalent: wrap the raw pointer
  as `Retained<ProtocolObject<dyn MTLTexture>>` (with the existing
  `ManuallyDrop`/retain discipline — `Frame::drop` owns the retain).
- `objc::rc::autoreleasepool` → `objc2::rc::autoreleasepool`.

### `syphon-wgpu/src/lib.rs`
- `use metal::*;` → objc2-metal imports. `metal_device: Option<Device>`,
  `metal_queue: Option<CommandQueue>` retype.
- `create_iosurface_texture` — a second copy of the IOSurface texture
  creator; same treatment as syphon-metal's (dedupe if practical).
- `publish_metal_texture(tex_ptr, cmd_ptr)` takes raw `id` pointers from
  `syphon-core` — unchanged at the boundary; only the objc2 side changes.

### `syphon-core`
- `Frame::metal_texture_ptr()` / `publish_metal_texture` traffic in raw
  `*mut Object`/`id` pointers — likely unchanged, but confirm no `objc 0.2`
  `Object` type leaks in the signatures (swap for `*mut AnyObject` /
  `objc2::runtime::AnyObject` if so).

## Cargo.toml changes

Both crates: drop `metal = "0.33"` and `objc = "0.2"`; add
`objc2 = "0.6"`, `objc2-metal = "0.3"`, `objc2-foundation = "0.3"` (match
the versions wgpu-hal 29 already resolves). Keep `io-surface`,
`core-foundation`, `core-graphics` (no metal/paste in those).

## Verification

1. `cargo build`/`test` on macOS — unit tests cover device creation + pool.
2. `cargo tree -i paste` in a downstream (rustjay-engine) → gone.
3. **Hardware:** run a real Syphon sender→receiver and confirm the zero-copy
   IOSurface blit shows correct, non-garbage frames. This is the step that
   can't be automated here; it's the actual acceptance test.
4. Bump versions (breaking internal types) and republish; rustjay picks it
   up by version.
