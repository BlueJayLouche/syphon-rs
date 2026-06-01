# Troubleshooting Guide

## Common Issues

### 1. "Library not loaded" / "image not found"

**Error:**
```
dyld[PID]: Library not loaded: @rpath/Syphon.framework/Versions/A/Syphon
  Referenced from: /path/to/your/app
  Reason: image not found
```

**Causes & Solutions:**

#### A. Framework not in search path

**Solution 1:** Use local framework (recommended)
```bash
# Copy framework to shared location
cp -R ~/Downloads/Syphon.framework ./syphon-lib/

# Build with rpath
cargo build
```

**Solution 2:** Set environment variable
```bash
export DYLD_FRAMEWORK_PATH=/Library/Frameworks
cargo run
```

**Solution 3:** Modify system framework (not recommended)
```bash
sudo install_name_tool -id \
    /Library/Frameworks/Syphon.framework/Versions/A/Syphon \
    /Library/Frameworks/Syphon.framework/Syphon
```

#### B. Framework has wrong install name

Check:
```bash
otool -D ./syphon-lib/Syphon.framework/Syphon
```

Should show:
```
@rpath/Syphon.framework/Versions/A/Syphon
```

Not:
```
@loader_path/../Frameworks/...
```

---

### 2. "Attempt to use unknown class" / Segmentation Fault

**Error:**
```
objc[PID]: Attempt to use unknown class 0x...
zsh: segmentation fault
```

**Cause:** Missing `autoreleasepool` in user code that calls ObjC APIs directly.

Syphon crate methods manage their own autorelease pools internally — you do **not** need
to wrap calls like `SyphonClient::connect()` or `receive_texture()`.

If you are writing your own ObjC interop in the same thread (e.g. calling `msg_send!`
directly), wrap those calls:

```rust
use objc::rc::autoreleasepool;

thread::spawn(move || {
    autoreleasepool(|| {
        // your own ObjC calls here
    });
});
```

---

### 3. Server not found / No servers discovered

**Error:**
```
No Syphon servers found. Make sure you have a server running.
```

**Causes & Solutions:**

#### A. Server not started yet

`SyphonServerDirectory::servers()` already handles timing internally:
- Returns immediately if any servers are already registered
- Otherwise sends `requestServerAnnounce` and waits up to 200ms for responses

If `servers()` returns empty, the server process is simply not running yet. Start the
server and call `servers()` again — no manual retry loop needed.

#### B. Different framework instances

Server and client using different Syphon.framework copies:

```bash
# Check both use same framework
otool -L server_binary | grep Syphon
otool -L client_binary | grep Syphon

# Should show same path
```

#### C. macOS privacy/security

Check System Preferences > Security & Privacy:
- Add both apps to Screen Recording if needed
- No permission dialogs blocked

---

### 4. High CPU Usage

**Problem:** Receive loop consuming 100% CPU.

**Best solution:** Switch to push-based delivery — zero CPU when idle:

```rust
let rx = input.connect_with_channel("Server")?;
while rx.recv().is_ok() {
    // woken exactly when a new frame arrives
    if let Some(texture) = input.receive_texture(&device, &queue) { ... }
}
```

**Alternative (polling):** Add a sleep to avoid busy-waiting:

```rust
while running.load(Ordering::SeqCst) {
    match client.try_receive() {
        Ok(Some(frame)) => { /* process */ }
        Ok(None) => thread::sleep(Duration::from_millis(1)),
        Err(e) => log::warn!("Error: {}", e),
    }
}
```

---

### 5. Black Frames / No Video

**Causes:**

#### A. Wrong pixel format

Syphon uses BGRA on macOS. Check your texture format:

```rust
// Correct
desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);

// Wrong
desc.set_pixel_format(MTLPixelFormat::RGBA8Unorm);
```

#### B. Coordinate system mismatch

wgpu uses top-left origin, Metal uses bottom-left. The syphon-wgpu crate handles this automatically for servers.

#### C. Texture not committed

Ensure Metal command buffer is committed:

```rust
cmd_buf.commit();
```

#### D. Server not actually sending

Test with the official Simple Client app to verify the server is working.

---

## Zero-Copy Specific Issues

### 6. Zero-Copy Not Working (Server)

**Problem:** `is_zero_copy()` returns false.

**Check:**

1. **Same GPU:** Rendering and Syphon must use same Metal device
2. **Shared storage mode:** Texture must be `MTLStorageMode::Shared`
3. **Framework issue:** Try `new_with_framework_device()` fallback
4. **Texture format:** Must be `Bgra8Unorm`
5. **Texture usage:** Must have `COPY_SRC`

```rust
// Check zero-copy status
if !output.is_zero_copy() {
    log::warn!("Falling back to CPU readback - check GPU compatibility");
}
```

### 7. Zero-Copy Client: "Failed to create Metal texture"

**Problem:** `create_texture_from_iosurface()` returns `None`

**Causes & Solutions:**

#### A. Metal context not properly initialized

```rust
// Ensure Metal is available
let metal_ctx = MetalContext::system_default()
    .expect("Metal not available on this system");

// Or from raw device
let metal_ctx = unsafe { 
    MetalContext::from_raw_device(device) 
};
```

#### B. IOSurface format mismatch

```rust
// Check IOSurface properties
let surface_ref = frame.iosurface_ref();
// Verify pixel format is BGRA
```

#### C. Dimension mismatch

```rust
// Always use frame dimensions
let texture = metal_ctx.create_texture_from_iosurface(
    surface,
    frame.width,   // Use frame dimensions, not your target size
    frame.height
)?;
```

#### D. Device compatibility

Some Metal devices don't support IOSurface-backed textures:

```rust
// Check device supports shared storage
let device = metal_ctx.device();
// Most Apple Silicon and modern discrete GPUs support this
```

### 8. Zero-Copy Client: Texture appears corrupted

**Problem:** Texture shows garbage or wrong colors

**Causes:**

#### A. Wrong pixel format assumption

Syphon frames are **BGRA8Unorm**, not RGBA:

```rust
// Correct - texture format matches IOSurface
let desc = MTLTextureDescriptor::new();
desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);

// If you need RGBA, you must convert on GPU
```

#### B. Premature texture release

The Metal texture shares memory with the IOSurface. Don't drop the `Frame` while using the texture:

```rust
// WRONG - texture becomes invalid after frame is dropped
let texture = {
    let frame = client.try_receive()?.unwrap();
    metal_ctx.create_texture_from_iosurface(&frame.iosurface(), w, h)?
}; // frame dropped here - texture now invalid!
use_texture(&texture); // CRASH or corruption

// CORRECT - keep frame alive
let frame = client.try_receive()?.unwrap();
let texture = metal_ctx.create_texture_from_iosurface(&frame.iosurface(), w, h)?;
use_texture(&texture); // OK - frame still alive
```

### 9. Zero-Copy Performance Issues

**Problem:** Zero-copy is slow or not as fast as expected

**Solutions:**

#### A. Check you're not accidentally using CPU readback

```rust
// BAD - CPU readback
let data = frame.to_vec()?; // Copies to CPU!
let texture = upload_to_gpu(&data);

// GOOD - Zero-copy
let texture = metal_ctx.create_texture_from_iosurface(
    frame.iosurface(), w, h
)?; // No CPU copies!
```

#### B. Profile texture creation time

```rust
let start = Instant::now();
let texture = metal_ctx.create_texture_from_iosurface(...)?;
println!("Texture creation: {:?}", start.elapsed());
// Should be < 100µs for zero-copy
```

#### C. Check for implicit synchronization

IOSurface operations may trigger GPU synchronization:

```rust
// Ensure you're not blocking on GPU work
// Use triple buffering for server (IOSurfacePool handles this)
// For client, just ensure you're not holding old frames
```

---

## Client-Specific Issues

### 10. Client receiving but frames are blank/black

**Causes:**

#### A. Frame lock failure

```rust
// Try receiving again if first attempt fails
match client.try_receive() {
    Ok(Some(mut frame)) => {
        match frame.to_vec() {
            Ok(data) => process(data),
            Err(e) => {
                // Frame might be in use by server
                log::warn!("Failed to lock frame, will retry");
            }
        }
    }
    Ok(None) => { /* No new frame */ }
    Err(e) => log::error!("Receive error: {}", e),
}
```

#### B. Stride mismatch

Some servers use padded rows:

```rust
let bytes_per_row = frame.bytes_per_row();
let expected_stride = frame.width * 4;
if bytes_per_row != expected_stride as usize {
    log::warn!("Stride mismatch: {} vs {}", bytes_per_row, expected_stride);
    // Handle padded rows
}
```

### 11. Client disconnects randomly

**Cause:** Connection lost or server stopped

**Solution:**

```rust
// Check connection status
if !client.is_connected() {
    log::warn!("Connection lost, attempting to reconnect...");
    // Try to reconnect or notify user
}

// In receive loop, handle errors gracefully
loop {
    match client.try_receive() {
        Ok(Some(frame)) => process(frame),
        Ok(None) => thread::sleep(Duration::from_millis(1)),
        Err(e) => {
            log::error!("Receive failed: {}", e);
            if !client.is_connected() {
                break; // Exit loop, connection lost
            }
        }
    }
}
```

### 12. Memory leaks in long-running clients

**Cause:** Not releasing frames promptly.

The Syphon crate internalises autorelease pool management. The main thing to watch:
drop `Frame` objects as soon as you are done with them so the IOSurface goes back to
the server's pool. Holding frames across multiple loop iterations can exhaust the pool.

Using push-based delivery (`connect_with_channel`) means your code wakes only when a
frame is available, which naturally keeps one frame in flight at a time.

---

## Build Errors

### "SyphonClient class not found"

Framework not linked correctly. Check `build.rs`:

```rust
println!("cargo:rustc-link-search=framework=./syphon-lib");
```

### "Cannot find objc crate"

Add to Cargo.toml:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc = "0.2"
```

### "wgpu_hal not found"

For zero-copy wgpu, you need wgpu with the Metal feature enabled. The crate currently
targets wgpu 29.x:

```toml
[dependencies]
wgpu = { version = "29", features = ["metal"] }
```

All `wgpu-hal` interop is isolated in `syphon-wgpu/src/metal_interop.rs` — when
upgrading wgpu, that is the only file that needs updating.

---

## Debugging Tips

### Enable Debug Logging

```bash
RUST_LOG=debug cargo run
```

For more verbose output:

```bash
RUST_LOG=trace cargo run
```

### Check Framework Loading

```bash
# Verify binary links to framework
otool -L target/debug/your_app | grep Syphon

# Check rpath settings
otool -l target/debug/your_app | grep -A2 LC_RPATH
```

### Verify Framework Content

```bash
# Check framework exists and has correct structure
ls -la ./syphon-lib/Syphon.framework/

# Check install name
otool -D ./syphon-lib/Syphon.framework/Syphon
```

### Test with Simple Examples

```bash
# Terminal 1: Start server
cargo run --example simple_server

# Terminal 2: Connect client  
cargo run --example simple_client

# For zero-copy testing:
cargo run --example metal_client -- "Server Name"
```

### Check GPU Availability

```rust
use syphon_core::metal_device;

let gpus = metal_device::available_devices();
for gpu in &gpus {
    println!("GPU: {}", gpu.name);
    println!("  Low power: {}", gpu.is_low_power);
    println!("  Unified memory: {}", gpu.has_unified_memory);
    println!("  Default: {}", gpu.is_default);
}
```

### Verify Zero-Copy Path

```rust
// For server
let output = SyphonWgpuOutput::new(...)?;
println!("Zero-copy active: {}", output.is_zero_copy());

// For client
let start = Instant::now();
let texture = metal_ctx.create_texture_from_iosurface(...)?;
println!("Texture creation took: {:?}", start.elapsed());
// If < 100µs, it's zero-copy. If > 1ms, likely CPU fallback
```

---

## Platform-Specific Issues

### macOS 14+ (Sonoma)

No known issues, but ensure:
- Latest Syphon framework (3.0+)
- Xcode Command Line Tools installed

### Apple Silicon (M1/M2/M3)

Should work natively. If issues:
- Build for arm64: `cargo build --target aarch64-apple-darwin`
- Check Rosetta not interfering

### Intel Mac

Should work. If issues:
- Build for x86_64: `cargo build --target x86_64-apple-darwin`

### Multi-GPU Systems (MacBook Pro with dGPU)

Ensure rendering and Syphon use same GPU:

```rust
use syphon_core::metal_device;

metal_device::validate_device_match(render_device, syphon_device)?;
```

---

## Getting Help

Before filing an issue:

1. ✅ Test with `simple_server`/`simple_client` examples
2. ✅ For zero-copy issues, test with `metal_client` example
3. ✅ Enable debug logging
4. ✅ Check framework installation
5. ✅ If using direct ObjC calls, verify autoreleasepool usage in those paths

Include in bug report:
- macOS version (`sw_vers`)
- GPU model (About This Mac)
- Full error output
- Minimal reproduction code
- Framework version
- Whether issue is with server, client, or both

---

## Quick Diagnostic Checklist

```bash
# 1. Framework exists
ls ./syphon-lib/Syphon.framework/Syphon

# 2. Correct install name
otool -D ./syphon-lib/Syphon.framework/Syphon

# 3. Binary links correctly
otool -L target/debug/your_app | grep Syphon

# 4. Syphon available at runtime
cargo run --example simple_server  # Should print "Syphon is available!"

# 5. Discovery works
cargo run --example simple_client  # Should find server

# 6. Zero-copy client works
cargo run --example metal_client -- "Server Name"

# 7. Connection works
# (Run server and client examples simultaneously)
```

---

## Common Error Messages Reference

| Error | Likely Cause | Solution |
|-------|--------------|----------|
| `Library not loaded` | Framework path issue | Check DYLD_FRAMEWORK_PATH or rpath |
| `Attempt to use unknown class` | Missing autoreleasepool in your own ObjC code | Wrap direct ObjC calls in `autoreleasepool` |
| `AmbiguousServerName` | Multiple servers share the same name | Use `connect_by_info()` with a UUID |
| `Server not found` | Server not running or framework mismatch | Start the server, check framework paths |
| `Failed to lock IOSurface` | Frame in use or dropped | Retry, ensure frame not dropped early |
| `Failed to create Metal texture` | Format/dimension mismatch | Check BGRA8Unorm format, use frame dimensions |
| `Image upside-down` | Missing Y-flip | Server: use compute shader; Client: flip in shader |
| `Black frames` | Server not sending or wrong format | Test with Simple Client app |
| `High CPU usage` | Busy-waiting in receive loop | Use `connect_with_channel()` for push delivery, or add `thread::sleep` |
