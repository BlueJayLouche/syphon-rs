# Syphon

Rust bindings and utilities for [Syphon](https://syphon.v002.info/) — an open source macOS framework for sharing video between applications in real-time, with **zero-copy GPU-to-GPU support** for both sending and receiving.

## Overview

This workspace provides Rust crates for integrating Syphon frame sharing into your applications with maximum performance:

- **Zero-copy publishing** from wgpu/Metal textures to Syphon
- **Zero-copy receiving** as wgpu or Metal textures (IOSurface GPU blit, no CPU copies)
- **Push-based delivery** via `newFrameHandler` — no polling
- **Native BGRA format** — no format conversion overhead
- **UUID-based server discovery** — unambiguous even when names collide

## Features

- ✅ **Zero-Copy GPU Transfer**: Both send and receive without CPU readback
- ✅ **Push-Based Frame Delivery**: `connect_with_channel()` wakes your thread on every new frame
- ✅ **UUID Server Lookup**: `connect_by_info()` is unambiguous — no more name collisions
- ✅ **Explicit Transfer Status**: `publish()` returns `PublishStatus` — no silent CPU fallbacks
- ✅ **Direct Metal Interop**: Access frames as `MTLTexture` for custom rendering
- ✅ **IOSurface Backing**: Shared GPU memory for efficient texture sharing
- ✅ **Triple-Buffering**: Prevents GPU stalls with automatic surface pooling
- ✅ **wgpu Integration**: High-level API for wgpu applications
- ✅ **Private Server Support**: `ServerOptions { is_private: true }` hides from directory
- ✅ **wgpu-hal Isolation**: All version-specific interop in one file (`metal_interop.rs`)

## Crates

| Crate | Description |
|-------|-------------|
| [`syphon-core`](./syphon-core/) | Core Objective-C bindings — `SyphonServer`, `SyphonClient`, `Frame` |
| [`syphon-metal`](./syphon-metal/) | Metal/IOSurface utilities — `MetalContext`, `IOSurfacePool` |
| [`syphon-wgpu`](./syphon-wgpu/) | High-level wgpu integration — `SyphonWgpuOutput`, `SyphonWgpuInput` |
| [`syphon-examples`](./syphon-examples/) | Minimal example applications |

## Requirements

- macOS 10.13+ (required for Syphon framework)
- Metal-capable GPU
- Xcode Command Line Tools

## Quick Start

```bash
git clone https://github.com/BlueJayLouche/syphon-rs.git
cd syphon-rs

# Build the workspace (Syphon.framework is bundled in syphon-core/frameworks)
cargo build --workspace --release

# Run examples
cargo run --example wgpu_sender --release      # Send from wgpu
cargo run --example metal_client --release     # Receive with Metal (zero-copy)
```

## Usage

### Server: Publishing from wgpu

```rust
use syphon_wgpu::{SyphonWgpuOutput, PublishStatus};

// Create the Syphon output
let mut output = SyphonWgpuOutput::new(
    "My App",      // Server name visible to clients
    &wgpu_device,  // Your wgpu device
    &wgpu_queue,   // Your wgpu queue
    1920,          // Width
    1080           // Height
).expect("Failed to create Syphon output");

// Each frame, publish your rendered Bgra8Unorm texture.
// publish() returns a status so silent CPU fallbacks are impossible.
match output.publish(&render_texture, &wgpu_device, &wgpu_queue) {
    PublishStatus::ZeroCopy   => { /* GPU-to-GPU blit, ~0% CPU */ }
    PublishStatus::CpuFallback => log::warn!("CPU fallback — check Metal setup"),
    PublishStatus::NoClients  => { /* No receivers connected */ }
    PublishStatus::PoolExhausted => log::warn!("Increase pool_size in SyphonOutputConfig"),
}
```

#### Advanced: private servers and custom pool size

```rust
use syphon_wgpu::{SyphonWgpuOutput, SyphonOutputConfig, ServerOptions};

let config = SyphonOutputConfig {
    pool_size: 4,                             // default: 3
    server_options: ServerOptions { is_private: true }, // hidden from directory
};
let mut output = SyphonWgpuOutput::new_with_config(
    "Hidden App", &device, &queue, 1920, 1080, config
)?;
```

### Client: Push-Based Delivery (Recommended)

No polling — the receiver channel wakes your thread exactly when a new frame is available:

```rust
use syphon_wgpu::SyphonWgpuInput;

let mut input = SyphonWgpuInput::new(&device, &queue);
let rx = input.connect_with_channel("My App")?;

thread::spawn(move || {
    while rx.recv().is_ok() {
        if let Some(texture) = input.receive_texture(&device, &queue) {
            // texture is Bgra8Unorm, GPU-blitted zero-copy on Metal
        }
    }
});
```

### Client: UUID-Based Connection (Unambiguous)

When multiple servers might share a name, list them and connect by `ServerInfo`:

```rust
use syphon_core::SyphonServerDirectory;
use syphon_wgpu::SyphonWgpuInput;

let servers = SyphonServerDirectory::servers();
// servers() returns immediately if any are already announced;
// otherwise it triggers requestServerAnnounce and waits up to 200ms.

if let Some(info) = servers.iter().find(|s| s.app_name == "Resolume") {
    let mut input = SyphonWgpuInput::new(&device, &queue);
    input.connect_by_info(info)?;   // matched by UUID — never ambiguous
}
```

### Client: Receiving with Direct Metal (Zero-Copy)

For maximum performance when integrating into Metal-based applications:

```rust
use syphon_core::SyphonClient;
use syphon_metal::MetalContext;

let metal_ctx = MetalContext::system_default().expect("Metal not available");
let client = SyphonClient::connect("My App").expect("Failed to connect");

loop {
    if let Ok(Some(frame)) = client.try_receive() {
        // ZERO-COPY: Metal texture aliasing the IOSurface GPU memory
        let texture = metal_ctx.create_texture_from_iosurface(
            frame.iosurface(), frame.width, frame.height
        ).expect("Failed to create texture");
        // Format is BGRA8Unorm — no conversion needed
        render_with_metal_texture(&texture);
    }
}
```

### Client: Basic Frame Access (CPU Readback)

For simple use cases where you need raw pixel data:

```rust
use syphon_core::SyphonClient;

let client = SyphonClient::connect("My App")?;
if let Ok(Some(mut frame)) = client.try_receive() {
    println!("Frame: {}x{}", frame.width, frame.height);
    let pixel_data: Vec<u8> = frame.to_vec()?; // CPU copy — use sparingly
}
```

## Architecture

### Zero-Copy Data Flow

```
SENDER                                  RECEIVER
┌─────────────────────┐                ┌─────────────────────┐
│   wgpu/Metal App    │                │   wgpu/Metal App    │
│                     │                │                     │
│  ┌───────────────┐  │                │  ┌───────────────┐  │
│  │ wgpu Texture  │  │                │  │ wgpu/Metal    │  │
│  │ (Bgra8Unorm)  │  │                │  │ Texture       │  │
│  └───────┬───────┘  │                │  └───────┬───────┘  │
│          │ GPU blit │                │          ▲ GPU blit │
│          ▼          │                │          │          │
│  ┌───────────────┐  │                │  ┌───────────────┐  │
│  │  IOSurface    │◄─┼────────────────┼──┤  IOSurface    │  │
│  │  (shared mem) │  │  Syphon.framework  │  (shared mem) │  │
│  └───────────────┘  │                │  └───────────────┘  │
└─────────────────────┘                └─────────────────────┘
        No CPU copies end-to-end (Metal backend)
```

### Key Components

1. **syphon-core**: Core FFI bindings to Syphon.framework
   - `SyphonServer` — Publishes frames; supports `ServerOptions`
   - `SyphonClient` — Receives frames; push (`connect_with_channel`) or poll (`try_receive`)
   - `SyphonServerDirectory` — Fast server discovery via `requestServerAnnounce`
   - `Frame` — Contains IOSurface reference

2. **syphon-metal**: Metal interop utilities
   - `MetalContext` — Metal device/queue management
   - `IOSurfacePool` — Efficient triple-buffered surface reuse
   - `create_texture_from_iosurface()` — Zero-copy texture aliasing

3. **syphon-wgpu**: High-level wgpu integration
   - `SyphonWgpuOutput` — Publish wgpu textures; returns `PublishStatus`
   - `SyphonWgpuInput` — Receive to wgpu textures; GPU blit on Metal
   - `metal_interop` — All `wgpu-hal` version-specific code isolated here

## Format

This crate uses **native macOS BGRA8Unorm format** throughout:

- **Output**: Render to `Bgra8Unorm` textures and publish directly
- **Input**: Received textures are `Bgra8Unorm` (no conversion)

This eliminates all format conversion overhead and provides maximum performance.

## Performance

| Operation | CPU Overhead | Latency | Throughput |
|-----------|-------------|---------|------------|
| **Server Zero-Copy** (publish) | ~0% | ~1ms | 60-240 FPS @ 4K |
| **Client wgpu Input** (Metal blit) | ~0% | ~1ms | 60-240 FPS @ 4K |
| **Client Metal** (IOSurface alias) | ~0% | ~1ms | 60-240 FPS @ 4K |
| **CPU Fallback** (any path) | ~5-10% | ~5ms | 30-60 FPS @ 4K |

Zero-copy eliminates GPU→CPU transfers, CPU→GPU uploads, staging buffers, and format conversion.

## Examples

```bash
# Server example — wgpu zero-copy sender
cargo run --example wgpu_sender --release

# Client example — Direct Metal (zero-copy, fastest)
cargo run --example metal_client --release -- "Server Name"

# Simple client example
cargo run --example simple_client --release
```

## Upgrading wgpu

All `wgpu-hal` Metal API calls are isolated in `syphon_metal::wgpu_interop`
(`syphon-metal/src/lib.rs`). When upgrading wgpu, edit only that module.

**Current version: wgpu 29.** Key constraints to keep in mind for future upgrades:

- `as_hal` returns `Option<impl Deref<Target = A::Device>>` directly (no closure).
- `Queue::as_raw()` does not exist — the internal `MTLCommandQueue` is inaccessible.
  The blit path uses `MetalContext`'s own command queue plus `device.poll()` for ordering.
- `Device::raw_device()` returns `&Retained<ProtocolObject<dyn MTLDevice>>` and
  `Texture::raw_handle()` returns `&ProtocolObject<dyn MTLTexture>` — these are
  `objc2-metal` types, which this workspace uses natively, so extraction is a
  plain `clone()`/borrow. Keep the workspace's `objc2-metal` version in sync
  with the one wgpu-hal resolves.

## Linking from your own crate

Add `syphon-wgpu` (or `syphon-core`) and it builds with no further setup —
`syphon-core` reassembles its bundled `Syphon.framework` in its `OUT_DIR` and
emits the framework search path and link libraries, which propagate to you
normally.

**Runtime search paths do not propagate, though.** `cargo:rustc-link-arg`
applies only to the emitting package's own targets, so the `-rpath` that
`syphon-core` adds for its own tests and examples never reaches your binary.
Without one, `dyld` falls back to whatever is in `/Library/Frameworks` — and if
that copy predates Apple Silicon it is x86_64-only, which aborts the launch:

```
dyld[…]: Library not loaded: @rpath/Syphon.framework/Versions/A/Syphon
  Reason: … (mach-o file, but is an incompatible architecture (have 'x86_64', need 'arm64'))
```

`dyld` resolves `@rpath` against the first *match*, not the first *loadable*
match, so a stale system copy earlier in the search path wins over a perfectly
good bundled one.

So each crate that produces a **binary** linking Syphon — every leaf binary, and
any library whose own tests or examples link it — needs a `build.rs` of its own.
Because `syphon-core` declares `links = "Syphon"`, it exports the directory it
linked against as `DEP_SYPHON_FRAMEWORK_DIR`:

```rust
// build.rs
fn main() {
    #[cfg(target_os = "macos")]
    {
        // The framework syphon-core linked against. Listed first so a stale
        // x86_64-only copy in /Library/Frameworks cannot shadow it.
        if let Ok(dir) = std::env::var("DEP_SYPHON_FRAMEWORK_DIR") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
        }
        println!("cargo:rustc-link-arg=-Wl,-rpath,/Library/Frameworks");
        // So a packaged .app can carry its own copy:
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path/../Frameworks");

        println!("cargo:rerun-if-changed=build.rs");
        println!("cargo:rerun-if-env-changed=DEP_SYPHON_FRAMEWORK_DIR");
    }
}
```

Cargo sets `DEP_*` variables for **direct** dependents only. If a crate needs
the rpath but has no Syphon code of its own — a binary that reaches Syphon
through an intermediate library, say — give it a dependency anyway so the
metadata is visible, and comment it so nobody prunes it as unused:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
# Not used in code: makes syphon-core's `links = "Syphon"` metadata
# (DEP_SYPHON_FRAMEWORK_DIR) visible to build.rs, which needs it for the rpath.
syphon-core = "0.4"
```

`syphon-examples/build.rs` is a working copy of this pattern.

### Shipping an app bundle

The `DEP_SYPHON_FRAMEWORK_DIR` rpath points into `target/`, so it is a
development convenience — `cargo clean` invalidates it. A distributable `.app`
should carry the framework itself, which the `@executable_path` rpath above
then finds:

```bash
cp -R syphon-lib/Syphon.framework MyApp.app/Contents/Frameworks/
codesign --force --deep --sign - MyApp.app
```

Consuming from crates.io rather than a checkout? Take the framework from the
crate's build directory instead of `syphon-lib/`:

```bash
cp -R "$(find target/release/build -type d -name Syphon.framework | head -1)" \
      MyApp.app/Contents/Frameworks/
```

## Documentation

- **[ZERO_COPY_IMPLEMENTATION.md](./ZERO_COPY_IMPLEMENTATION.md)** — Complete technical details
- **[CHANGELOG.md](./CHANGELOG.md)** — Version history

## Syphon Framework

`Syphon.framework` ships bundled in `syphon-core/frameworks/`, so no extra setup
is needed to build. To refresh it, the framework source is tracked as a submodule:

```bash
git submodule update --init syphon-lib/Syphon-Framework
```

## Troubleshooting

### "framework 'Syphon' not found" at build time

`syphon-core` bundles the framework, so this should not happen from a normal
`cargo build`. If it does, the payload is missing or unreadable — check that
`syphon-core/frameworks/Versions/A/Syphon` exists. As a stopgap you can point
the linker at the in-repo copy:

```bash
export DYLD_FRAMEWORK_PATH="$PWD/syphon-lib"
```

### "Library not loaded: @rpath/Syphon.framework" at launch

The binary has no rpath that resolves to a loadable framework. Two causes:

1. **Your crate has no `build.rs` emitting an rpath.** See
   [Linking from your own crate](#linking-from-your-own-crate) — this is the
   common case, since `syphon-core` cannot add one on your behalf.
2. **A stale copy in `/Library/Frameworks` is shadowing the good one.** If the
   error says `incompatible architecture (have 'x86_64', need 'arm64')`, an old
   installer left a pre-Apple-Silicon build there:

   ```bash
   # Should list arm64; a bare `x86_64` is the stale case
   lipo -archs /Library/Frameworks/Syphon.framework/Versions/A/Syphon
   ```

   Replace it with a [current
   release](https://github.com/Syphon/Syphon-Framework/releases) or remove it
   and let the bundled framework be found. Installing system-wide is never
   required, and an outdated copy there will break other Syphon apps too.

### Zero-copy not working (Server)

1. Use the Metal backend (`wgpu::Backends::METAL`)
2. Texture format must be `Bgra8Unorm`
3. Texture must have `COPY_SRC` usage

```bash
RUST_LOG=info cargo run --example wgpu_sender
```

`publish()` returns `PublishStatus::CpuFallback` if the zero-copy path is unavailable.

### Multiple servers with the same name

`connect()` returns `SyphonError::AmbiguousServerName`. Use `connect_by_info()` instead:

```rust
let servers = SyphonServerDirectory::servers();
let info = servers.iter().find(|s| s.uuid == "known-uuid").unwrap();
client.connect_by_info(info)?;
```

## License

Licensed under the MIT License — see [LICENSE](./LICENSE) for details.
The bundled Syphon.framework is licensed under the BSD 3-Clause License.

## Links

- [Syphon Official Website](https://syphon.v002.info/)
- [Syphon Framework GitHub](https://github.com/Syphon/Syphon-Framework)

## Note

Syphon is macOS only. For cross-platform video sharing consider
[Spout](https://spout.zeal.co/) (Windows) or [NDI](https://ndi.tv/) (cross-platform).
