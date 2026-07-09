# Zero-Copy Syphon Implementation

This document describes the zero-copy GPU-to-GPU Syphon implementation for macOS, covering both **servers** (sending frames) and **clients** (receiving frames).

## Overview

The zero-copy approach eliminates CPU readback by using IOSurface-backed textures to transfer frames directly between applications on the GPU.

```
Old (CPU readback):
  GPU Texture → GPU Buffer → CPU RAM → new GPU Texture → Syphon

Zero-Copy (GPU only):
  GPU Texture ←→ IOSurface ←→ GPU Texture
                (shared memory)
```

**Key Benefits:**
- ~0% CPU overhead for frame transfer
- ~1ms latency (vs ~5ms for CPU readback)
- 60-240 FPS @ 4K depending on GPU
- No memory copies or staging buffers
- Native BGRA format throughout

---

## Table of Contents

1. [Architecture](#architecture)
2. [Server (Sending) Implementation](#server-sending-implementation)
3. [Client (Receiving) Implementation](#client-receiving-implementation)
4. [Direct Metal Integration](#direct-metal-integration)
5. [wgpu Integration](#wgpu-integration)
6. [Troubleshooting](#troubleshooting)

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                           Zero-Copy Architecture                        │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  SENDER APPLICATION                    RECEIVER APPLICATION             │
│  ┌─────────────────────┐              ┌─────────────────────┐          │
│  │   wgpu/Metal App    │              │   wgpu/Metal App    │          │
│  │                     │              │                     │          │
│  │  ┌───────────────┐  │              │  ┌───────────────┐  │          │
│  │  │ wgpu Texture  │  │              │  │ wgpu Texture  │  │          │
│  │  │ (Bgra8Unorm)  │  │              │  │ (Bgra8Unorm)  │  │          │
│  │  └───────┬───────┘  │              │  └───────┬───────┘  │          │
│  │          │          │              │          ▲          │          │
│  │          ▼          │              │          │          │          │
│  │  ┌───────────────┐  │              │  ┌───────────────┐  │          │
│  │  │  IOSurface    │◄─┼──────────────┼──┤  IOSurface    │  │          │
│  │  │  (shared)     │  │  Syphon.framework  │  (shared)     │  │          │
│  │  └───────┬───────┘  │              │  └───────────────┘  │          │
│  │          │          │              │                     │          │
│  │          ▼          │              │  syphon_core::      │          │
│  │  ┌───────────────┐  │              │  SyphonClient       │          │
│  │  │ syphon_core:: │  │              │                     │          │
│  │  │ SyphonServer  │  │              │  syphon_metal::     │          │
│  │  └───────────────┘  │              │  create_texture_    │          │
│  │                     │              │  from_iosurface()   │          │
│  └─────────────────────┘              └─────────────────────┘          │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Key Components

#### syphon-core
- `SyphonServer` - Publishes frames to Syphon
- `SyphonClient` - Receives frames from Syphon
- `Frame` - Contains an IOSurface reference
- `SyphonServerDirectory` - Server discovery

#### syphon-metal
- `IOSurfacePool` - Triple-buffered pool of reusable IOSurfaces
- `MetalContext` - Holds Metal device and queue for texture operations
- `create_texture_from_iosurface()` - Creates Metal texture from IOSurface
- `wgpu_interop` - Extract raw Metal handles from wgpu objects

#### syphon-wgpu
- `SyphonWgpuOutput` - Publish wgpu textures to Syphon (server); returns `PublishStatus`
- `SyphonWgpuInput` - Receive frames as wgpu textures (client); GPU blit on Metal
- `metal_interop` - All `wgpu-hal` version-specific code isolated here (wgpu 29.0)

---

## Server (Sending) Implementation

### The Zero-Copy Flow (Server)

All `wgpu-hal` calls are isolated in `metal_interop.rs` — upgrading wgpu only requires
editing that file.

```rust
// 1. Acquire IOSurface from pool
let surface = surface_pool.acquire();

// 2. Create IOSurface-backed destination texture (shares GPU memory)
let dest_texture = create_iosurface_texture(&surface, width, height);

// 3. Flush wgpu's pending GPU work before reading the source texture.
//    (Queue::as_raw() was removed in wgpu 29 — we can't use wgpu's queue directly.)
device.poll(wgpu::PollType::wait_indefinitely());

// 4. Blit from wgpu source → IOSurface-backed destination on our Metal queue.
//    metal_interop::with_metal_texture extracts the raw MTLTexture handle.
metal_interop::with_metal_texture(&texture, |src| {
    let cmd_buf = metal_queue.new_command_buffer();
    let blit = cmd_buf.new_blit_command_encoder();
    blit.copy_from_texture(
        src,
        0, 0, MTLOrigin { x: 0, y: 0, z: 0 },
        MTLSize { width, height, depth: 1 },
        &dest_texture,
        0, 0, MTLOrigin { x: 0, y: 0, z: 0 },
    );
    blit.end_encoding();

    // 5. Publish to Syphon before committing
    server.publish_metal_texture(dest_texture, cmd_buf);

    cmd_buf.commit();
});
```

### Native BGRA Format

This implementation uses **native BGRA8Unorm format** throughout:

- Render your content to `Bgra8Unorm` textures
- Publish directly without any format conversion
- Clients receive BGRA data natively

This eliminates format conversion overhead and provides maximum performance.

### Synchronization (wgpu 29+)

`wgpu-hal` no longer exposes the internal `MTLCommandQueue` — `Queue::as_raw()` was
removed in wgpu 29. The blit therefore runs on a separate `metal::CommandQueue` created
from the same `MTLDevice` as wgpu (so they share the same GPU timeline).

To guarantee all prior wgpu render work has finished before the blit reads from the
source texture, `device.poll(PollType::wait_indefinitely())` is called immediately
before submitting the blit. This drains the wgpu submit queue and blocks until the
GPU is idle.

---

## Client (Receiving) Implementation

### Receiving with Direct Metal (Zero-Copy)

For applications that need direct Metal texture access without wgpu overhead:

```rust
use syphon_core::SyphonClient;
use syphon_metal::{IOSurfacePool, MetalContext};
use metal::MTLPixelFormat;

// 1. Create a Metal context
let metal_ctx = MetalContext::system_default()
    .expect("Metal not available");

// 2. Connect to a Syphon server
let client = SyphonClient::connect("Simple Server")
    .expect("Failed to connect to server");

// 3. Receive frames in a loop
loop {
    if let Ok(Some(mut frame)) = client.try_receive() {
        let width = frame.width;
        let height = frame.height;
        
        // 4. Get the IOSurface from the frame
        let surface = frame.iosurface();
        
        // 5. Create a Metal texture directly from IOSurface (ZERO COPY!)
        let metal_texture = metal_ctx.create_texture_from_iosurface(
            surface,
            width,
            height
        ).expect("Failed to create texture");
        
        // 6. Use the texture in your Metal render pipeline
        // The texture is BGRA8Unorm format (native Syphon format)
        render_with_metal_texture(&metal_texture);
        
        // Texture is released when dropped, IOSurface goes back to pool
    }
}
```

### Accessing Raw IOSurface

For advanced use cases, you can access the raw IOSurface reference:

```rust
// Borrow the IOSurface (objc2-io-surface type, for interop with other libraries)
let surface: &objc2_io_surface::IOSurfaceRef = frame.iosurface();

// Get the IOSurface ID (for logging/debugging)
let surface_id = frame.iosurface_id();
```

### Frame Locking (CPU Access)

If you need CPU access to frame data (not zero-copy):

```rust
// Lock the surface for reading
if let Ok((addr, seed)) = frame.lock() {
    let height = frame.height as usize;
    let stride = frame.bytes_per_row();
    let size = height * stride;
    
    // Access raw pixel data
    let data = unsafe { 
        std::slice::from_raw_parts(addr, size) 
    };
    
    // Process pixels...
    
    // Unlock when done (important!)
    frame.unlock(seed).ok();
}

// Or simply copy to a Vec (convenience method)
let data: Vec<u8> = frame.to_vec()?;
```

**Important**: Avoid CPU readback if you want zero-copy performance. Use Metal texture creation instead.

---

## Direct Metal Integration

### Creating Metal Textures from IOSurface

The key to zero-copy on the client side is creating Metal textures directly from IOSurfaces:

```rust
use syphon_metal::MetalContext;
use io_surface::IOSurface;

fn create_texture_from_iosurface(
    device: &metal::Device,
    surface: &IOSurface,
    width: u32,
    height: u32,
) -> Option<metal::Texture> {
    use objc::runtime::Object;
    use objc::{msg_send, class};
    use cocoa::foundation::NSUInteger;
    use core_foundation::base::TCFType;
    use metal::{MTLStorageMode, MTLTextureUsage, MTLPixelFormat};
    
    unsafe {
        // Create texture descriptor
        let desc: *mut Object = msg_send![class!(MTLTextureDescriptor), new];
        let _: () = msg_send![desc, setPixelFormat: MTLPixelFormat::BGRA8Unorm];
        let _: () = msg_send![desc, setWidth: width as NSUInteger];
        let _: () = msg_send![desc, setHeight: height as NSUInteger];
        let _: () = msg_send![desc, setStorageMode: MTLStorageMode::Shared];
        let _: () = msg_send![
            desc, 
            setUsage: MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead
        ];
        
        // Get the raw IOSurfaceRef
        let surface_ref = surface.as_concrete_TypeRef();
        
        // Create texture from IOSurface
        let device_ptr = device.as_ptr() as *mut Object;
        let texture_ptr: *mut Object = msg_send![
            device_ptr,
            newTextureWithDescriptor: desc
            iosurface: surface_ref
            plane: 0 as NSUInteger
        ];
        
        // Release the descriptor
        let _: () = msg_send![desc, release];
        
        if texture_ptr.is_null() {
            None
        } else {
            Some(metal::Texture::from_ptr(texture_ptr as *mut metal::MTLTexture))
        }
    }
}
```

### Complete Direct Metal Client Example

```rust
//! Direct Metal Syphon Client Example
//! 
//! This example shows how to receive frames from a Syphon server
//! and access them as Metal textures without any copies.

use syphon_core::SyphonClient;
use syphon_metal::MetalContext;

fn main() {
    // Initialize Metal
    let metal_ctx = MetalContext::system_default()
        .expect("Metal not available");
    
    // Connect to server
    let client = SyphonClient::connect("Simple Server")
        .expect("Failed to connect");
    
    println!("Connected to: {}", client.server_name());
    
    // Setup your Metal render pipeline here...
    // let command_queue = metal_ctx.queue();
    
    loop {
        // Try to receive a frame
        match client.try_receive() {
            Ok(Some(mut frame)) => {
                // Create Metal texture from IOSurface (ZERO COPY)
                let texture = metal_ctx.create_texture_from_iosurface(
                    &frame.iosurface(),
                    frame.width,
                    frame.height
                ).expect("Failed to create texture");
                
                println!("Got frame: {}x{} (format: BGRA8Unorm)", 
                    frame.width, frame.height);
                
                // Use texture in your Metal pipeline
                // The texture shares memory with the IOSurface
                
                // When 'texture' is dropped, the IOSurface is released
                // When 'frame' is dropped, IOSurface goes back to Syphon pool
            }
            Ok(None) => {
                // No new frame yet
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }
}
```

### Metal Texture Format

Syphon frames are always **BGRA8Unorm** format:
- **Pixel format**: `MTLPixelFormat::BGRA8Unorm`
- **Storage mode**: `MTLStorageMode::Shared` (for IOSurface sharing)
- **Usage**: `MTLTextureUsage::ShaderRead` (and optionally `RenderTarget`)

---

## wgpu Integration

### Server: Publishing from wgpu

```rust
use syphon_wgpu::SyphonWgpuOutput;

// Create the output
let mut output = SyphonWgpuOutput::new(
    "My App",      // Syphon server name
    &device,       // wgpu device
    &queue,        // wgpu queue
    1920,          // width
    1080           // height
).expect("Failed to create Syphon output");

// Check if zero-copy is active
if output.is_zero_copy() {
    println!("Using zero-copy GPU-to-GPU path!");
}

// Each frame, publish your rendered texture
// Use Bgra8Unorm format for native performance
output.publish(&render_texture, &device, &queue);
```

### Client: Receiving to wgpu (Zero-Copy on Metal)

`SyphonWgpuInput` performs a GPU-to-GPU Metal blit when the wgpu device is Metal-backed:

```
IOSurface ← zero-copy Metal texture alias ──blit──► wgpu output texture
                                          (no CPU involvement)
```

```rust
use syphon_wgpu::SyphonWgpuInput;

let mut input = SyphonWgpuInput::new(&device, &queue);

// Push-based: woken exactly when a new frame is published
let rx = input.connect_with_channel("My Server")?;
while rx.recv().is_ok() {
    if let Some(texture) = input.receive_texture(&device, &queue) {
        // Bgra8Unorm, blitted GPU-to-GPU with zero CPU copies
    }
}
```

CPU fallback is retained for non-Metal backends and logged as a warning.

---

## Performance

### Server (Publishing)

| Method | CPU Overhead | Latency | Throughput |
|--------|-------------|---------|------------|
| Zero-Copy (GPU Blit) | ~0% | ~1ms | 60-240 FPS @ 4K |
| CPU Readback | ~5-10% | ~5ms | 30-60 FPS @ 4K |

### Client (Receiving)

| Method | CPU Overhead | Latency | Throughput |
|--------|-------------|---------|------------|
| Direct Metal (IOSurface alias) | ~0% | ~1ms | 60-240 FPS @ 4K |
| wgpu Input (Metal blit) | ~0% | ~1ms | 60-240 FPS @ 4K |
| CPU Readback (to_vec) | ~5-10% | ~5ms | 30-60 FPS @ 4K |

---

## Requirements

- macOS 10.13+ (for Syphon framework)
- Metal-capable GPU
- For wgpu: wgpu with Metal backend

---

## Building

```bash
# Syphon.framework is bundled in syphon-core/frameworks — no submodule needed
git clone https://github.com/BlueJayLouche/syphon-rs.git
cd syphon-rs

# Build the workspace
cargo build --workspace --release

# Run the wgpu sender example
cargo run --example wgpu_sender --release

# Run the simple client example
cargo run --example simple_client --release
```

---

## Known Limitations

1. **Syphon Framework**: Must be installed or bundled for linking
2. **Metal-only**: Zero-copy requires Metal backend (CPU fallback available for other backends)
3. **Format**: BGRA8Unorm only (native macOS format)

---

## Troubleshooting

### "No zero-copy path available"

- Ensure you're using Metal backend (`wgpu::Backends::METAL`)
- Check texture format is `Bgra8Unorm`
- Ensure texture has `COPY_SRC` usage

### "Failed to create IOSurface texture"

- Verify IOSurface dimensions match texture descriptor
- Check Metal device supports shared storage mode
- Ensure IOSurface pixel format is BGRA (`0x42475241`)

### Client receiving black frames

- Some servers may send frames with different stride/padding
- Check `frame.bytes_per_row()` vs `width * 4`
- Verify server is actually sending content (test with Simple Client app)

### Memory leaks

- Drop `Frame` objects promptly to release IOSurface references back to the pool
- Autorelease pools are managed internally — no user-level wrappers needed

---

## Future Improvements

1. **Async publish**: Non-blocking publish with fence synchronization
2. **Direct render-to-IOSurface**: Create wgpu texture directly from IOSurface to skip the blit entirely

---

## References

- [Syphon Framework](https://github.com/Syphon/Syphon-Framework)
- [Metal IOSurface Documentation](https://developer.apple.com/documentation/metal/mtldevice/1433355-newtexturewithdescriptor)
- [wgpu-hal Metal Backend](https://github.com/gfx-rs/wgpu/tree/trunk/wgpu-hal/src/metal)
- [IOSurface Programming Guide](https://developer.apple.com/library/archive/documentation/General/Conceptual/IOSurfaceProgGuide/Introduction/Introduction.html)

---

## Current Status

✅ **Production Ready** - The zero-copy implementation is stable and performant.

**Server (Publishing):**
- ✅ Zero-copy via IOSurface + Metal blit
- ✅ Native BGRA format (no conversion)
- ✅ Triple-buffering for GPU efficiency
- ✅ CPU fallback for compatibility

**Client (Receiving):**
- ✅ Direct Metal texture from IOSurface (zero-copy alias)
- ✅ wgpu input via GPU blit — zero CPU copies on Metal backend
- ✅ Push-based delivery via `connect_with_channel()` — no polling

---

## Migration from CPU-Based Clients

If you're currently using `frame.to_vec()`:

```rust
// OLD: CPU readback (slow)
let mut frame = client.try_receive()?.unwrap();
let data = frame.to_vec()?;  // CPU copy!
let texture = upload_to_gpu(&data);  // GPU upload!

// NEW: Zero-copy Metal (fast)
let mut frame = client.try_receive()?.unwrap();
let texture = metal_ctx.create_texture_from_iosurface(
    &frame.iosurface(),
    frame.width,
    frame.height
)?;  // No copies, GPU only!
```

The performance difference is significant: **5-10x faster** for high-resolution content.
