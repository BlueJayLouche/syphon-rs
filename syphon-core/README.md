# syphon-core

Core Rust bindings to [Syphon](https://syphon.v002.info/) — the open-source macOS
framework for sharing video frames between applications in real time with
zero-copy GPU efficiency.

This crate provides safe wrappers around the Syphon Objective-C API:
`SyphonServer`, `SyphonClient`, `Frame`, and `SyphonServerDirectory`, plus
helpers for IOSurface and Metal device discovery.

> **macOS only.** Requires `Syphon.framework` to be available at link/run time
> (see [Requirements](#requirements)).

## Features

- **`SyphonServer`** — publish IOSurface-backed frames; optional private servers
  via `ServerOptions { is_private: true }`.
- **`SyphonClient`** — connect by name or by `ServerInfo` (UUID, never ambiguous),
  receive frames push-based with `newFrameHandler`.
- **`SyphonServerDirectory`** — enumerate available servers.
- **Zero-copy access** — `Frame::iosurface()` exposes the shared GPU memory so you
  can alias it as a Metal texture (see [`syphon-metal`](https://crates.io/crates/syphon-metal)).

## Example

```rust,no_run
use syphon_core::{SyphonServer, SyphonClient, SyphonServerDirectory};

// Publish frames
let server = SyphonServer::new("My App", 1920, 1080)?;
server.publish_iosurface(&my_surface)?;

// Discover servers
for info in SyphonServerDirectory::servers() {
    println!("Found: {} ({})", info.name, info.app_name);
}

// Connect and receive
let client = SyphonClient::connect("Resolume Arena")?;
if let Some(frame) = client.try_receive()? {
    println!("{}x{}", frame.width, frame.height);
}
# Ok::<(), syphon_core::SyphonError>(())
```

## Requirements

- macOS 10.13+ with a Metal-capable GPU
- Xcode Command Line Tools
- **`Syphon.framework`** installed in `/Library/Frameworks` (or otherwise on the
  framework search path). This ships with most Syphon-enabled apps, or build it
  from the [Syphon-Framework](https://github.com/Syphon/Syphon-Framework) repo.
  The build script also probes a sibling `syphon-lib/Syphon.framework` for local
  workspace development.

## Feature flags

- `logging` — cap log output at `warn` in release builds.
- `max_perf` — compile out all logging in release builds.

## License

MIT. Part of the [syphon-rs](https://github.com/BlueJayLouche/syphon-rs) workspace.
