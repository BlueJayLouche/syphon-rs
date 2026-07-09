//! # Syphon wgpu Integration - Zero-Copy Edition
//! 
//! High-performance, zero-copy GPU-to-GPU Syphon integration for wgpu applications.
//! 
//! ## Overview
//! 
//! This crate provides:
//! - `SyphonWgpuOutput` - Publish wgpu-rendered frames to Syphon clients
//! - `SyphonWgpuInput` - Receive frames from Syphon servers as wgpu textures
//! 
//! Both use IOSurface-backed textures for zero-copy GPU transfer.
//! 
//! ## Native BGRA Format
//! 
//! This crate uses native macOS BGRA8Unorm format throughout. When rendering for Syphon output,
//! use `wgpu::TextureFormat::Bgra8Unorm`. When receiving from Syphon, you'll get BGRA data
//! directly without any format conversion.
//!
//! ## Usage
//! 
//! ### Output (Server)
//! ```ignore
//! use syphon_wgpu::SyphonWgpuOutput;
//! 
//! let mut output = SyphonWgpuOutput::new(
//!     "My App", &device, &queue, 1920, 1080
//! ).expect("Failed to create Syphon output");
//! 
//! output.publish(&render_texture, &device, &queue);
//! ```
//!
//! ### Input (Client)
//! ```ignore
//! use syphon_wgpu::SyphonWgpuInput;
//!
//! let mut input = SyphonWgpuInput::new(&device, &queue);
//! input.connect("Simple Server").unwrap();
//!
//! if let Some(texture) = input.receive_texture(&device, &queue) {
//!     // Texture is Bgra8Unorm (native Syphon format)
//! }
//! ```

pub use syphon_core::{SyphonServer, SyphonClient, SyphonError, Result, ServerInfo, ServerOptions};

/// Result returned by [`SyphonWgpuOutput::publish`].
///
/// Check this to confirm which code path was used — never assume zero-copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishStatus {
    /// Frame published via zero-copy GPU blit through IOSurface — no CPU involved.
    ZeroCopy,
    /// Frame published via CPU readback (Metal interop unavailable).
    CpuFallback,
    /// No clients connected; frame skipped (no work done).
    NoClients,
    /// All IOSurfaces in the pool were in-flight; frame dropped.
    ///
    /// Increase `pool_size` in [`SyphonOutputConfig`] if this occurs regularly.
    PoolExhausted,
}

/// Configuration for [`SyphonWgpuOutput`].
#[derive(Debug, Clone)]
pub struct SyphonOutputConfig {
    /// Number of IOSurfaces to pre-allocate for triple-(or more) buffering.
    ///
    /// Must be ≥ 1. Default: `3` (suitable for ≤ 120 Hz with typical GPU latency).
    /// Increase to `4`–`5` at very high frame rates or if [`PublishStatus::PoolExhausted`]
    /// appears in logs.
    pub pool_size: usize,

    /// Core Syphon server options (e.g. `is_private`).
    pub server_options: ServerOptions,
}

impl Default for SyphonOutputConfig {
    fn default() -> Self {
        Self { pool_size: 3, server_options: ServerOptions::default() }
    }
}

// Input module for receiving frames
pub mod input;
pub use input::SyphonWgpuInput;

// All wgpu-hal Metal interop lives in syphon_metal::wgpu_interop —
// update only that module when upgrading wgpu.
#[cfg(target_os = "macos")]
use syphon_metal::wgpu_interop;

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyObject, ProtocolObject};
#[cfg(target_os = "macos")]
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLOrigin, MTLPixelFormat, MTLRegion,
    MTLSize, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

/// High-level wgpu-to-Syphon output with zero-copy GPU transfer
/// 
/// This implementation uses IOSurface-backed textures to transfer frames directly
/// from wgpu to Syphon without CPU readback.
/// 
/// ## Format
/// 
/// Uses native BGRA8Unorm format. Render your content to a Bgra8Unorm texture
/// and publish it directly.
pub struct SyphonWgpuOutput {
    server: SyphonServer,
    width: u32,
    height: u32,
    #[cfg(target_os = "macos")]
    surface_pool: syphon_metal::IOSurfacePool,
    #[cfg(target_os = "macos")]
    frame_count: u64,
    #[cfg(target_os = "macos")]
    use_zero_copy: bool,
    #[cfg(target_os = "macos")]
    metal_ctx: syphon_metal::MetalContext,
}

// SAFETY: `SyphonWgpuOutput` wraps a `SyphonServer` (Send+Sync, see its SAFETY
// comment), a `metal::Device` and `metal::CommandQueue` (both thread-safe per
// Apple's Metal docs), and an `IOSurfacePool` which holds `io_surface::IOSurface`
// objects. IOSurface is a reference-counted CoreFoundation type whose
// retain/release operations are thread-safe. The mutable fields `frame_count`,
// `use_zero_copy`, `width`, and `height` are written only in `new` or `publish`
// which should not be called concurrently (the caller must serialise publishes).
// Sync is acceptable because all constituent types are individually safe to
// access from shared references on concurrent threads.
#[cfg(target_os = "macos")]
unsafe impl Send for SyphonWgpuOutput {}
#[cfg(target_os = "macos")]
unsafe impl Sync for SyphonWgpuOutput {}

impl SyphonWgpuOutput {
    /// Create a new Syphon output using the default [`SyphonOutputConfig`].
    pub fn new(
        name: &str,
        wgpu_device: &wgpu::Device,
        wgpu_queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_with_config(name, wgpu_device, wgpu_queue, width, height, SyphonOutputConfig::default())
    }

    /// Create a new Syphon output with explicit configuration.
    ///
    /// Use this when you need a non-default pool size (e.g. at >120 Hz).
    pub fn new_with_config(
        name: &str,
        wgpu_device: &wgpu::Device,
        _wgpu_queue: &wgpu::Queue,
        width: u32,
        height: u32,
        config: SyphonOutputConfig,
    ) -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            Self::new_macos(name, wgpu_device, _wgpu_queue, width, height, config)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (name, wgpu_device, _wgpu_queue, width, height, config);
            Err(SyphonError::NotAvailable)
        }
    }

    #[cfg(target_os = "macos")]
    fn new_macos(
        name: &str,
        wgpu_device: &wgpu::Device,
        _wgpu_queue: &wgpu::Queue,
        width: u32,
        height: u32,
        config: SyphonOutputConfig,
    ) -> Result<Self> {
        // Try to get the Metal device from wgpu for zero-copy.
        let ctx_opt = syphon_metal::MetalContext::from_wgpu_device(wgpu_device);
        let use_zero_copy = ctx_opt.is_some();

        let metal_ctx = match ctx_opt {
            Some(ctx) => {
                log::info!("SyphonWgpuOutput: Using zero-copy GPU-to-GPU path");
                ctx
            }
            None => {
                log::warn!("SyphonWgpuOutput: Metal interop failed, falling back to CPU readback");
                syphon_metal::MetalContext::system_default()
                    .ok_or_else(|| SyphonError::CreateFailed(
                        "Failed to get Metal device".to_string()
                    ))?
            }
        };

        // Create the Syphon server with the Metal device and server options.
        let device_ptr =
            metal_ctx.device() as *const ProtocolObject<dyn MTLDevice> as *mut AnyObject;
        let server = SyphonServer::new_with_name_and_device_and_options(
            name, device_ptr, width, height, config.server_options.clone()
        )?;

        // Fallback mode does not use the pool (CPU path allocates per-frame).
        let pool_size = if use_zero_copy { config.pool_size.max(1) } else { 0 };
        let surface_pool = syphon_metal::IOSurfacePool::new(width, height, pool_size);

        log::info!(
            "SyphonWgpuOutput created: {}x{} ({})",
            width, height,
            if use_zero_copy { "zero-copy" } else { "CPU fallback" }
        );

        Ok(Self {
            server,
            width,
            height,
            surface_pool,
            frame_count: 0,
            use_zero_copy,
            metal_ctx,
        })
    }
    
    /// Publish a texture to Syphon.
    ///
    /// Returns a [`PublishStatus`] indicating which path was used. Check it —
    /// never assume zero-copy is active, especially after device changes.
    ///
    /// The texture must be in `Bgra8Unorm` format.
    pub fn publish(
        &mut self,
        texture: &wgpu::Texture,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> PublishStatus {
        #[cfg(target_os = "macos")]
        {
            if self.server.client_count() == 0 {
                return PublishStatus::NoClients;
            }

            self.frame_count += 1;

            if self.use_zero_copy {
                self.publish_zero_copy(texture, device, queue)
            } else {
                self.publish_cpu_fallback(texture, device, queue);
                PublishStatus::CpuFallback
            }
        }

        #[cfg(not(target_os = "macos"))]
        PublishStatus::NoClients
    }
    
    #[cfg(target_os = "macos")]
    fn publish_zero_copy(
        &mut self,
        texture: &wgpu::Texture,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> PublishStatus {
        let surface = match self.surface_pool.acquire() {
            Some(s) => s,
            None => {
                log::warn!(
                    "[SyphonWgpuOutput] IOSurface pool exhausted — frame dropped. \
                     Consider increasing pool_size in SyphonOutputConfig."
                );
                return PublishStatus::PoolExhausted;
            }
        };

        let mut published = false;

        // In wgpu 29, the internal MTLCommandQueue is no longer accessible via
        // wgpu-hal. We use our own metal_queue and poll wgpu first to ensure
        // all prior GPU rendering is complete before we blit.
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        objc2::rc::autoreleasepool(|_| {
            wgpu_interop::with_metal_texture(texture, |src_texture| {
                let Some(src_texture) = src_texture else { return };

                let Some((cmd_buf, dest_texture)) = self.metal_ctx.blit_to_iosurface(
                    src_texture, &surface, self.width, self.height
                ) else { return };

                // Notify Syphon before committing so it can reference
                // the command buffer for synchronisation.
                let tex_ptr = &*dest_texture as *const ProtocolObject<dyn MTLTexture> as *mut AnyObject;
                let cmd_ptr = &*cmd_buf as *const ProtocolObject<dyn MTLCommandBuffer> as *mut AnyObject;
                unsafe { self.server.publish_metal_texture(tex_ptr, cmd_ptr) };

                cmd_buf.commit();
                published = true;
            });
        });

        // Triple-buffering ensures the GPU has finished with this surface before
        // it's reused — return it even if publish failed so we don't leak it.
        self.surface_pool.release(surface);

        if published { PublishStatus::ZeroCopy } else { PublishStatus::CpuFallback }
    }
    
    #[cfg(target_os = "macos")]
    fn publish_cpu_fallback(&mut self, texture: &wgpu::Texture, device: &wgpu::Device, queue: &wgpu::Queue) {
        // CPU readback fallback implementation
        // This is the stable but slower path
        
        let buffer_size = (self.width * self.height * 4) as u64;
        
        // Create staging buffer
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Syphon Staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        
        // Copy texture to buffer
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Syphon Copy"),
        });
        
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        
        queue.submit(std::iter::once(encoder.finish()));
        
        // Wait for GPU
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        
        // Map and upload
        let buffer_slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result.is_ok());
        });
        
        // Wait for map (with timeout)
        let start = std::time::Instant::now();
        let mut ready = false;
        while start.elapsed().as_millis() < 10 {
            if let Ok(true) = rx.try_recv() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
            let _ = device.poll(wgpu::PollType::Poll);
        }
        
        if ready {
            let data = buffer_slice.get_mapped_range();
            
            // Check if we have actual data
            if data.iter().any(|&b| b != 0) {
                // Upload directly without flip - native BGRA
                let upload = unsafe {
                    let desc = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                        MTLPixelFormat::BGRA8Unorm,
                        self.width as usize,
                        self.height as usize,
                        false,
                    );
                    desc.setStorageMode(MTLStorageMode::Managed);
                    desc.setUsage(MTLTextureUsage::ShaderRead);
                    self.metal_ctx.device().newTextureWithDescriptor(&desc)
                };

                if let (Some(mtl_texture), Some(cmd_buf)) =
                    (upload, self.metal_ctx.queue().commandBuffer())
                {
                    unsafe {
                        mtl_texture.replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                            MTLRegion {
                                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                                size: MTLSize {
                                    width: self.width as usize,
                                    height: self.height as usize,
                                    depth: 1,
                                },
                            },
                            0,
                            std::ptr::NonNull::new_unchecked(data.as_ptr() as *mut _),
                            (self.width * 4) as usize,
                        );

                        let texture_ptr = &*mtl_texture as *const ProtocolObject<dyn MTLTexture> as *mut AnyObject;
                        let cmd_buf_ptr = &*cmd_buf as *const ProtocolObject<dyn MTLCommandBuffer> as *mut AnyObject;
                        self.server.publish_metal_texture(texture_ptr, cmd_buf_ptr);
                    }

                    cmd_buf.commit();
                }
            }
            
            drop(data);
            buffer.unmap();
        }
    }
    
    /// Get the number of connected clients
    pub fn client_count(&self) -> usize {
        self.server.client_count()
    }
    
    /// Check if any clients are connected
    pub fn has_clients(&self) -> bool {
        self.server.client_count() > 0
    }
    
    /// Get the server name
    pub fn name(&self) -> &str {
        self.server.name()
    }
    
    /// Get dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    
    /// Check if zero-copy is being used
    #[cfg(target_os = "macos")]
    pub fn is_zero_copy(&self) -> bool {
        self.use_zero_copy
    }
    
    /// Check if zero-copy is being used (non-macOS always returns false)
    #[cfg(not(target_os = "macos"))]
    pub fn is_zero_copy(&self) -> bool {
        false
    }
}

/// List available Syphon servers
pub fn list_servers() -> Vec<String> {
    syphon_core::SyphonServerDirectory::servers()
        .into_iter()
        .map(|info| info.name)
        .collect()
}

/// Check if Syphon is available on this system
pub fn is_available() -> bool {
    syphon_core::is_available()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_availability() {
        println!("Syphon available: {}", is_available());
    }
}
