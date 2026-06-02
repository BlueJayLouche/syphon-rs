# syphon-metal

Metal and IOSurface utilities for [Syphon](https://syphon.v002.info/) on macOS —
the plumbing for zero-copy GPU-to-GPU frame sharing.

Built on top of [`syphon-core`](https://crates.io/crates/syphon-core). If you use
wgpu, prefer the higher-level [`syphon-wgpu`](https://crates.io/crates/syphon-wgpu)
crate; reach for this one when integrating directly with Metal.

> **macOS only.** Requires `Syphon.framework` at link/run time.

## What it provides

- **`MetalContext`** — wrap the system default Metal device, or borrow the device
  underlying a wgpu instance (`from_wgpu_device`, behind the `wgpu` feature) for
  zero-copy interop.
- **`IOSurfacePool`** — a small pool of reusable IOSurfaces to avoid per-frame
  allocation and GPU stalls.
- **`create_texture_from_iosurface`** — alias an IOSurface as an `MTLTexture`
  (BGRA8Unorm) with no CPU copy.

## Example

```rust,ignore
use syphon_metal::{IOSurfacePool, MetalContext};

let ctx = MetalContext::system_default().expect("Metal not available");
let pool = IOSurfacePool::new(1920, 1080, 3);

let surface = pool.acquire().unwrap();
let texture = ctx.create_texture_from_iosurface(&surface, 1920, 1080)?;
# Ok::<(), syphon_core::SyphonError>(())
```

## Feature flags

- `wgpu` — enable wgpu/wgpu-hal interop (`MetalContext::from_wgpu_device`).

## Requirements

- macOS 10.13+ with a Metal-capable GPU
- `Syphon.framework` on the framework search path (typically `/Library/Frameworks`)

## License

MIT. Part of the [syphon-rs](https://github.com/BlueJayLouche/syphon-rs) workspace.
