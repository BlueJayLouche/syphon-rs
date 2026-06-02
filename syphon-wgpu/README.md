# syphon-wgpu

High-level, **zero-copy** [Syphon](https://syphon.v002.info/) integration for
[wgpu](https://wgpu.rs/) applications on macOS. Publish your rendered frames to
other apps, or receive frames as wgpu textures — GPU-to-GPU, no CPU readback.

Built on [`syphon-core`](https://crates.io/crates/syphon-core) and
[`syphon-metal`](https://crates.io/crates/syphon-metal).

> **macOS only.** Requires `Syphon.framework` at link/run time. Targets `wgpu 29`.

## Highlights

- **`SyphonWgpuOutput`** — publish a `Bgra8Unorm` wgpu texture each frame.
  `publish()` returns a `PublishStatus` so silent CPU fallbacks are impossible.
- **`SyphonWgpuInput`** — receive frames as wgpu textures, push-based via
  `connect_with_channel()` (no polling) or by UUID via `connect_by_info()`.
- **Native BGRA** end-to-end — no format conversion overhead.
- Triple-buffered IOSurface pool to avoid GPU stalls.

## Output (server)

```rust,no_run
use syphon_wgpu::{SyphonWgpuOutput, PublishStatus};

let mut output = SyphonWgpuOutput::new("My App", &device, &queue, 1920, 1080)
    .expect("failed to create Syphon output");

match output.publish(&render_texture, &device, &queue) {
    PublishStatus::ZeroCopy      => { /* GPU-to-GPU blit */ }
    PublishStatus::CpuFallback   => log::warn!("CPU fallback — check Metal setup"),
    PublishStatus::NoClients     => { /* no receivers connected */ }
    PublishStatus::PoolExhausted => log::warn!("increase pool_size"),
}
```

## Input (client, push-based)

```rust,no_run
use syphon_wgpu::SyphonWgpuInput;

let mut input = SyphonWgpuInput::new(&device, &queue);
let rx = input.connect_with_channel("My App")?;

while rx.recv().is_ok() {
    if let Some(texture) = input.receive_texture(&device, &queue) {
        // texture is Bgra8Unorm, GPU-blitted zero-copy
    }
}
# Ok::<(), syphon_core::SyphonError>(())
```

## Requirements

- macOS 10.13+ with a Metal-capable GPU
- `Syphon.framework` on the framework search path (typically `/Library/Frameworks`)
- `wgpu 29`

## Feature flags

- `logging` — cap log output at `warn` in release builds.
- `max_perf` — compile out all logging in release builds.

## License

MIT. Part of the [syphon-rs](https://github.com/BlueJayLouche/syphon-rs) workspace.
