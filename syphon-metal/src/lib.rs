//! # Syphon Metal
//!
//! Metal-specific utilities for Syphon, including IOSurface creation
//! and Metal texture interop for zero-copy GPU-to-GPU sharing.
//!
//! Built on `objc2-metal` / `objc2-io-surface` — the same bindings wgpu
//! itself uses, so wgpu interop is type-safe with no pointer punning.
//!
//! ## Usage
//!
//! ```ignore
//! use syphon_metal::{IOSurfacePool, MetalContext};
//!
//! // Create a Metal context from a wgpu device (for zero-copy)
//! let ctx = MetalContext::from_wgpu_device(&wgpu_device)?;
//!
//! // Create an IOSurface pool for efficient reuse
//! let pool = IOSurfacePool::new(1920, 1080, 3);
//!
//! // Get a surface and create a Metal texture from it
//! let surface = pool.acquire().unwrap();
//! let texture = ctx.create_texture_from_iosurface(&surface, 1920, 1080)?;
//! ```

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString};
#[cfg(target_os = "macos")]
use objc2_metal::{
    MTLCommandBuffer, MTLCommandQueue, MTLDevice, MTLOrigin, MTLPixelFormat, MTLSize,
    MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};

// Re-export the IOSurface types used in this crate's public API.
#[cfg(target_os = "macos")]
pub use objc2_io_surface::IOSurfaceRef;

/// An owned (retained) IOSurface.
#[cfg(target_os = "macos")]
pub type IOSurface = CFRetained<IOSurfaceRef>;

/// A pool of reusable IOSurfaces for efficient frame publishing
#[cfg(target_os = "macos")]
pub struct IOSurfacePool {
    width: u32,
    height: u32,
    surfaces: Vec<IOSurface>,
}

#[cfg(target_os = "macos")]
impl IOSurfacePool {
    /// Create a new pool with the specified capacity
    pub fn new(width: u32, height: u32, capacity: usize) -> Self {
        let mut surfaces = Vec::with_capacity(capacity);

        for _ in 0..capacity {
            if let Some(surface) = create_iosurface(width, height) {
                surfaces.push(surface);
            }
        }

        Self {
            width,
            height,
            surfaces,
        }
    }

    /// Acquire an IOSurface from the pool
    /// Returns None if all surfaces are in use
    pub fn acquire(&mut self) -> Option<IOSurface> {
        self.surfaces.pop()
    }

    /// Return an IOSurface to the pool
    pub fn release(&mut self, surface: IOSurface) {
        self.surfaces.push(surface);
    }

    /// Get pool capacity
    pub fn capacity(&self) -> usize {
        self.surfaces.capacity()
    }

    /// Get available surface count
    pub fn available(&self) -> usize {
        self.surfaces.len()
    }

    /// Get dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Stub implementation for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct IOSurfacePool {
    width: u32,
    height: u32,
}

#[cfg(not(target_os = "macos"))]
impl IOSurfacePool {
    pub fn new(width: u32, height: u32, _capacity: usize) -> Self {
        Self { width, height }
    }
    pub fn capacity(&self) -> usize { 0 }
    pub fn available(&self) -> usize { 0 }
    pub fn dimensions(&self) -> (u32, u32) { (self.width, self.height) }
}

/// Create a BGRA8 IOSurface with the specified dimensions
#[cfg(target_os = "macos")]
fn create_iosurface(width: u32, height: u32) -> Option<IOSurface> {
    use objc2_io_surface::{
        kIOSurfaceBytesPerElement, kIOSurfaceHeight, kIOSurfacePixelFormat, kIOSurfaceWidth,
    };

    let width_num = CFNumber::new_i64(width as i64);
    let height_num = CFNumber::new_i64(height as i64);
    let bytes_per_elem = CFNumber::new_i64(4); // RGBA8 = 4 bytes
    let pixel_format = CFNumber::new_i64(0x42475241); // 'BGRA'

    unsafe {
        let keys: [&CFString; 4] = [
            kIOSurfaceWidth,
            kIOSurfaceHeight,
            kIOSurfaceBytesPerElement,
            kIOSurfacePixelFormat,
        ];
        let values: [&CFNumber; 4] = [&width_num, &height_num, &bytes_per_elem, &pixel_format];
        let props = CFDictionary::from_slices(&keys, &values);
        IOSurfaceRef::new(props.as_opaque())
    }
}

/// Metal context for zero-copy interop with wgpu
///
/// This holds the Metal device and command queue for creating
/// IOSurface-backed textures and performing GPU blits.
#[cfg(target_os = "macos")]
pub struct MetalContext {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

#[cfg(target_os = "macos")]
impl MetalContext {
    /// Create a Metal context from an existing Metal device.
    ///
    /// Returns `None` if a command queue cannot be created.
    pub fn from_device(device: Retained<ProtocolObject<dyn MTLDevice>>) -> Option<Self> {
        let queue = device.newCommandQueue()?;
        Some(Self { device, queue })
    }

    /// Create a Metal context from the system default device
    pub fn system_default() -> Option<Self> {
        Self::from_device(objc2_metal::MTLCreateSystemDefaultDevice()?)
    }

    /// Get the Metal device
    pub fn device(&self) -> &ProtocolObject<dyn MTLDevice> {
        &self.device
    }

    /// Get the Metal command queue
    pub fn queue(&self) -> &ProtocolObject<dyn MTLCommandQueue> {
        &self.queue
    }

    /// Create a Metal texture that shares memory with the IOSurface,
    /// enabling zero-copy GPU-to-GPU transfers.
    pub fn create_texture_from_iosurface(
        &self,
        surface: &IOSurfaceRef,
        width: u32,
        height: u32,
    ) -> Option<Retained<ProtocolObject<dyn MTLTexture>>> {
        unsafe {
            let desc = MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width as usize,
                height as usize,
                false,
            );
            desc.setStorageMode(MTLStorageMode::Shared);
            desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);

            self.device
                .newTextureWithDescriptor_iosurface_plane(&desc, surface, 0)
        }
    }

    /// Encode a GPU blit from `src_texture` into an IOSurface-backed texture.
    ///
    /// Returns the **uncommitted** command buffer and the destination texture so
    /// the caller can reference both (e.g. hand them to Syphon's
    /// `publishFrameTexture:onCommandBuffer:`) before committing.
    ///
    /// Returns `None` if the destination texture or command buffer could not
    /// be created.
    #[allow(clippy::type_complexity)]
    pub fn blit_to_iosurface(
        &self,
        src_texture: &ProtocolObject<dyn MTLTexture>,
        dest_surface: &IOSurfaceRef,
        width: u32,
        height: u32,
    ) -> Option<(
        Retained<ProtocolObject<dyn MTLCommandBuffer>>,
        Retained<ProtocolObject<dyn MTLTexture>>,
    )> {
        let dest_texture = self.create_texture_from_iosurface(dest_surface, width, height)?;

        let cmd_buf = self.queue.commandBuffer()?;
        let blit_encoder = cmd_buf.blitCommandEncoder()?;
        unsafe {
            use objc2_metal::MTLBlitCommandEncoder;
            blit_encoder
                .copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                    src_texture,
                    0, // source slice
                    0, // source level
                    MTLOrigin { x: 0, y: 0, z: 0 },
                    MTLSize {
                        width: width as usize,
                        height: height as usize,
                        depth: 1,
                    },
                    &dest_texture,
                    0, // destination slice
                    0, // destination level
                    MTLOrigin { x: 0, y: 0, z: 0 },
                );
        }
        use objc2_metal::MTLCommandEncoder;
        blit_encoder.endEncoding();

        // The encoder retains referenced resources, so dest_texture stays alive
        // through GPU execution even if the caller drops it after commit.
        Some((cmd_buf, dest_texture))
    }
}

#[cfg(all(feature = "wgpu", target_os = "macos"))]
impl MetalContext {
    /// Create a Metal context from a wgpu device for zero-copy interop.
    ///
    /// Returns `None` when the wgpu device is not backed by Metal.
    pub fn from_wgpu_device(device: &wgpu::Device) -> Option<Self> {
        Self::from_device(wgpu_interop::extract_metal_device(device)?)
    }
}

/// Stub implementation for non-macOS platforms
#[cfg(not(target_os = "macos"))]
pub struct MetalContext;

#[cfg(not(target_os = "macos"))]
impl MetalContext {
    pub fn system_default() -> Option<Self> { None }
}

/// Helpers to extract Metal handles from wgpu objects using wgpu-hal.
///
/// **All `wgpu-hal` version-specific code lives here.**
/// When upgrading wgpu, edit only this module.
///
/// wgpu-hal 29 returns `objc2-metal` types directly, so these are safe
/// clones/borrows — no pointer punning.
#[cfg(all(feature = "wgpu", target_os = "macos"))]
pub mod wgpu_interop {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_metal::{MTLDevice, MTLTexture};

    /// Extract the Metal device backing a wgpu device.
    ///
    /// Returns `None` when the wgpu device is not backed by Metal.
    /// The device is retained, so it can be held independently of wgpu.
    pub fn extract_metal_device(
        device: &wgpu::Device,
    ) -> Option<Retained<ProtocolObject<dyn MTLDevice>>> {
        // SAFETY: as_hal only requires the device to be alive, which the
        // reference guarantees; the returned device is retained (cloned).
        let hal_guard = unsafe { device.as_hal::<wgpu_hal::api::Metal>() }?;
        Some(hal_guard.raw_device().clone())
    }

    /// Call `f` with the Metal texture backing a wgpu texture.
    ///
    /// `f` receives `None` when the texture is not Metal-backed.
    /// The reference is only valid for the duration of `f`.
    pub fn with_metal_texture<F, R>(texture: &wgpu::Texture, f: F) -> R
    where
        F: FnOnce(Option<&ProtocolObject<dyn MTLTexture>>) -> R,
    {
        // SAFETY: as_hal only requires the texture to be alive; the borrow
        // handed to `f` cannot outlive the hal guard.
        match unsafe { texture.as_hal::<wgpu_hal::api::Metal>() } {
            Some(hal_guard) => f(Some(hal_guard.raw_handle())),
            None => f(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_surface_pool() {
        let mut pool = IOSurfacePool::new(640, 480, 3);
        assert_eq!(pool.capacity(), 3);

        #[cfg(target_os = "macos")]
        {
            assert_eq!(pool.available(), 3);

            let surface = pool.acquire();
            assert!(surface.is_some());
            assert_eq!(pool.available(), 2);

            pool.release(surface.unwrap());
            assert_eq!(pool.available(), 3);
        }
    }

    #[test]
    fn test_metal_context_creation() {
        // This test only works on macOS with Metal available
        #[cfg(target_os = "macos")]
        {
            match MetalContext::system_default() { Some(_ctx) => {
                println!("Metal context created successfully");
            } _ => {
                println!("Metal not available on this system");
            }}
        }
    }
}
