//! Syphon wgpu input receiver
//!
//! ## Zero-copy path (default on Metal)
//!
//! When the wgpu device is backed by Metal, frames are transferred via a GPU
//! blit from the IOSurface-backed Metal texture directly into the output wgpu
//! texture — no CPU involvement at all.
//!
//! The output texture is kept alive across frames and initialized through
//! wgpu once on creation so that wgpu's texture-initialization tracking does
//! not zero it out before the first shader use.
//!
//! ## CPU fallback
//!
//! If the Metal HAL is unavailable (e.g. wgpu Vulkan/DX12), the frame is
//! locked on the CPU and uploaded via `queue.write_texture`.

use syphon_core::{SyphonClient, Result, ServerInfo};
#[cfg(target_os = "macos")]
use syphon_metal::wgpu_interop;
#[cfg(target_os = "macos")]
use objc2::runtime::ProtocolObject;
#[cfg(target_os = "macos")]
use objc2_metal::{
    MTLBlitCommandEncoder, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLOrigin,
    MTLSharedEvent, MTLSize, MTLTexture,
};
#[cfg(target_os = "macos")]
use syphon_metal::wgpu_interop::SharedEvent;

/// Cross-queue ordering for the receive path, in place of a CPU drain.
///
/// The output texture is written by Syphon's command queue and read by wgpu's.
/// Metal only orders command buffers within a single queue, so *both*
/// directions need an explicit fence and neither comes for free:
///
/// * read-after-write — wgpu must not sample the texture until the blit lands.
///   The blit signals `blit_done`; wgpu's next submit waits on it.
/// * write-after-read — the next blit must not overwrite the texture while
///   wgpu is still reading the last one. wgpu's submit signals `wgpu_done`;
///   the next blit's command buffer waits on it.
///
/// Only the second of those was ever covered before, and by draining wgpu on
/// the CPU. One counter drives both: frame `n` waits for `wgpu_done >= n - 1`,
/// signals `blit_done = n`, and asks wgpu to signal `wgpu_done = n`.
#[cfg(target_os = "macos")]
struct EventSync {
    blit_done: SharedEvent,
    wgpu_done: SharedEvent,
    /// Frames blitted through this path. Only ever increases: shared-event
    /// waits are `>=`, so a counter that never rewinds can never pass early.
    frame: u64,
}

#[cfg(target_os = "macos")]
impl EventSync {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        let blit_done = syphon_metal::wgpu_interop::new_shared_event(device)?;
        let wgpu_done = syphon_metal::wgpu_interop::new_shared_event(device)?;
        // Probe the queue once rather than discovering per frame that it
        // cannot carry events. Waiting for 0 is already satisfied, so the
        // staged wait costs the next submit nothing; the call also puts the
        // queue into strict ordering mode up front.
        if !syphon_metal::wgpu_interop::queue_wait_for_event(queue, &blit_done, 0) {
            log::warn!(
                "[SyphonWgpuInput] queue cannot carry shared events; using the CPU drain"
            );
            return None;
        }
        Some(Self { blit_done, wgpu_done, frame: 0 })
    }
}

/// What a blit command buffer fences against.
#[cfg(target_os = "macos")]
struct BlitSync<'a> {
    wgpu_done: &'a ProtocolObject<dyn MTLSharedEvent>,
    wait_value: u64,
    blit_done: &'a ProtocolObject<dyn MTLSharedEvent>,
    signal_value: u64,
}

/// Whether to order the receive path with shared events instead of the CPU
/// drain. Off unless `SYPHON_WGPU_SYNC=event`.
///
/// Both paths live in one binary deliberately. Comparing them across separate
/// builds is unreliable here: a live publisher intermittently emits black
/// frames, so a single run of a receive test proves nothing either way, and
/// the variants have to be interleaved within one process to be judged.
#[cfg(target_os = "macos")]
fn event_sync_enabled() -> bool {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        matches!(std::env::var("SYPHON_WGPU_SYNC").as_deref(), Ok("event"))
    })
}

pub struct SyphonWgpuInput {
    client: Option<SyphonClient>,
    connected_server: Option<String>,
    /// Persistent output texture — initialized via wgpu on creation so
    /// wgpu's init-tracking never zeroes it after an external Metal blit.
    output_texture: Option<wgpu::Texture>,
    output_width: u32,
    output_height: u32,
    /// Metal context created from wgpu's underlying Metal device.
    /// Present only when wgpu is backed by Metal.
    #[cfg(target_os = "macos")]
    metal_ctx: Option<syphon_metal::MetalContext>,
    /// GPU-side ordering with wgpu's queue. `None` falls back to the CPU drain.
    #[cfg(target_os = "macos")]
    sync: Option<EventSync>,
}

impl SyphonWgpuInput {
    /// Create a new input receiver.
    ///
    /// Extracts the underlying Metal device from `device` (if Metal-backed) so
    /// the zero-copy blit path is available immediately.
    pub fn new(device: &wgpu::Device, _queue: &wgpu::Queue) -> Self {
        #[cfg(target_os = "macos")]
        let metal_ctx = Self::build_metal_ctx(device);
        #[cfg(target_os = "macos")]
        let sync = if metal_ctx.is_some() && event_sync_enabled() {
            EventSync::new(device, _queue)
        } else {
            None
        };

        Self {
            client: None,
            connected_server: None,
            output_texture: None,
            output_width: 0,
            output_height: 0,
            #[cfg(target_os = "macos")]
            metal_ctx,
            #[cfg(target_os = "macos")]
            sync,
        }
    }

    #[cfg(target_os = "macos")]
    fn build_metal_ctx(device: &wgpu::Device) -> Option<syphon_metal::MetalContext> {
        let ctx = syphon_metal::MetalContext::from_wgpu_device(device);
        if ctx.is_none() {
            log::warn!("[SyphonWgpuInput] wgpu device is not Metal-backed; will use CPU fallback");
        }
        ctx
    }

    /// Connect to a Syphon server by display name.
    ///
    /// Returns [`SyphonError::AmbiguousServerName`] when multiple servers share
    /// the same name. In that case use [`connect_by_info`](Self::connect_by_info).
    pub fn connect(&mut self, server_name: &str) -> Result<()> {
        log::info!("[SyphonWgpuInput] Connecting to '{}'", server_name);
        let client = SyphonClient::connect(server_name)?;
        self.client = Some(client);
        self.connected_server = Some(server_name.to_string());
        log::info!("[SyphonWgpuInput] Connected");
        Ok(())
    }

    /// Connect using a [`ServerInfo`] obtained from `SyphonServerDirectory`.
    /// Matches by UUID — unambiguous even when names collide.
    pub fn connect_by_info(&mut self, info: &ServerInfo) -> Result<()> {
        log::info!("[SyphonWgpuInput] Connecting to '{}' (uuid={})", info.display_name(), info.uuid);
        let client = SyphonClient::connect_by_info(info)?;
        self.connected_server = Some(info.display_name().to_string());
        self.client = Some(client);
        log::info!("[SyphonWgpuInput] Connected");
        Ok(())
    }

    /// Connect with push-based delivery via a channel.
    ///
    /// Returns `((), receiver)`. The receiver yields `()` each time the server
    /// publishes a new frame — no polling needed. Call [`receive_texture`](Self::receive_texture)
    /// after waking on the channel.
    pub fn connect_with_channel(
        &mut self,
        server_name: &str,
    ) -> Result<std::sync::mpsc::Receiver<()>> {
        log::info!("[SyphonWgpuInput] Connecting to '{}' (push mode)", server_name);
        let (client, rx) = SyphonClient::connect_with_channel(server_name)?;
        self.connected_server = Some(server_name.to_string());
        self.client = Some(client);
        log::info!("[SyphonWgpuInput] Connected (push mode)");
        Ok(rx)
    }

    /// Connect by [`ServerInfo`] with push-based delivery.
    ///
    /// UUID-based — unambiguous even when names collide.
    pub fn connect_by_info_with_channel(
        &mut self,
        info: &ServerInfo,
    ) -> Result<std::sync::mpsc::Receiver<()>> {
        log::info!("[SyphonWgpuInput] Connecting to '{}' (uuid={}, push mode)", info.display_name(), info.uuid);
        let (client, rx) = SyphonClient::connect_by_info_with_channel(info)?;
        self.connected_server = Some(info.display_name().to_string());
        self.client = Some(client);
        log::info!("[SyphonWgpuInput] Connected (push mode)");
        Ok(rx)
    }

    pub fn disconnect(&mut self) {
        self.client = None;
        self.connected_server = None;
        self.output_texture = None;
        log::info!("[SyphonWgpuInput] Disconnected");
    }

    pub fn is_connected(&self) -> bool {
        self.client.as_ref().is_some_and(|c| {
            #[cfg(target_os = "macos")]
            { c.is_connected() }
            #[cfg(not(target_os = "macos"))]
            { true }
        })
    }

    /// Try to receive a frame into the persistent output texture.
    ///
    /// Returns `true` when a new frame was written; `false` when no new frame
    /// is available.  Access the result with [`output_texture`](Self::output_texture).
    ///
    /// On Metal, performs a GPU-to-GPU blit with zero CPU copies.
    /// On other backends, falls back to CPU upload.
    pub fn receive_texture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> bool {
        let client = match self.client.as_ref() {
            Some(c) => c,
            None => return false,
        };

        #[cfg(target_os = "macos")]
        {
            if !client.has_new_frame() { return false; }

            let mut frame = match client.try_receive() {
                Ok(Some(f)) => f,
                _ => return false,
            };

            let w = frame.width;
            let h = frame.height;

            // Create or resize the persistent output texture.
            // Zero-initialise via wgpu so its init-tracking marks it as "written"
            // — otherwise wgpu clears it before the first shader use, overwriting
            // any data the external Metal blit wrote.
            if self.output_texture.is_none() || self.output_width != w || self.output_height != h {
                log::info!("[SyphonWgpuInput] Creating output texture: {}x{}", w, h);
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("syphon_input"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Bgra8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_DST
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                // Write zeros through wgpu to mark the texture as initialized.
                let zeros = vec![0u8; (w * h * 4) as usize];
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &zeros,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(w * 4),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                self.output_texture = Some(tex);
                self.output_width = w;
                self.output_height = h;
            }

            let output = self.output_texture.as_ref().unwrap();

            // Attempt zero-copy GPU blit; fall back to CPU on failure.
            // Drain wgpu before the Metal blit so prior render work is done.
            let used_gpu = match (&self.metal_ctx, &mut self.sync) {
                (Some(ctx), Some(sync)) => {
                    Self::blit_with_events(&frame, output, ctx.queue(), sync, device, queue)
                }
                (Some(ctx), None) => {
                    crate::drain_wgpu_before_blit(device, "receive_texture");
                    Self::gpu_blit(&frame, output, ctx.queue(), None)
                }
                (None, _) => false,
            };

            if !used_gpu {
                log::warn!("[SyphonWgpuInput] GPU blit unavailable, using CPU fallback");
                let stride = frame.bytes_per_row() as u32;
                let data = match frame.to_vec() {
                    Ok(d) => d,
                    Err(e) => { log::warn!("[SyphonWgpuInput] CPU read failed: {}", e); return false; }
                };
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: output,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &data,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(stride),
                        rows_per_image: Some(h),
                    },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }

            true
        }

        #[cfg(not(target_os = "macos"))]
        { false }
    }

    /// The persistent output texture, valid after [`receive_texture`](Self::receive_texture)
    /// returns `true`.
    pub fn output_texture(&self) -> Option<&wgpu::Texture> {
        self.output_texture.as_ref()
    }

    /// GPU-to-GPU blit: Syphon frame texture → output wgpu texture, zero CPU copies.
    ///
    /// Uses `frame.metal_texture_ptr()` — the `id<MTLTexture>` returned by
    /// `SyphonMetalClient::newFrameImage`.  This texture was created on the
    /// *same* Metal device as the Syphon client, so no cross-device IOSurface
    /// re-wrapping is needed.
    ///
    /// Submitted on wgpu's own Metal command queue so Metal's queue-ordering
    /// guarantee ensures the blit completes before any subsequent wgpu commands
    /// that read `output`.
    /// GPU-to-GPU blit using a dedicated Metal command queue.
    ///
    /// Submitted on `MetalContext`'s own queue. With `sync` the command buffer
    /// fences itself against wgpu's queue; without it the caller must have
    /// called `crate::drain_wgpu_before_blit` first.
    ///
    /// Note that committing on wgpu's own queue instead (wgpu-hal 30 re-exposes
    /// it via `metal::Queue::as_raw()`) would not remove the need to fence:
    /// Metal lets independent command buffers within one queue overlap on the
    /// GPU, so sharing a queue orders nothing by itself.
    #[cfg(target_os = "macos")]
    fn gpu_blit(
        frame: &syphon_core::Frame,
        output: &wgpu::Texture,
        metal_queue: &ProtocolObject<dyn MTLCommandQueue>,
        sync: Option<&BlitSync<'_>>,
    ) -> bool {
        let frame_tex_ptr = frame.metal_texture_ptr();
        if frame_tex_ptr.is_null() {
            log::warn!("[SyphonWgpuInput] newFrameImage returned nil, cannot GPU blit");
            return false;
        }

        // SAFETY: the pointer is a valid id<MTLTexture> retained by `frame`
        // (released in Frame::drop); we only borrow it for the blit.
        let src: &ProtocolObject<dyn MTLTexture> =
            unsafe { &*(frame_tex_ptr as *const ProtocolObject<dyn MTLTexture>) };

        let mut ok = false;

        objc2::rc::autoreleasepool(|_| {
            wgpu_interop::with_metal_texture(output, |dst| {
                let Some(dst) = dst else { return };
                let Some(cmd) = metal_queue.commandBuffer() else { return };
                // Waits and signals must be encoded outside any encoder, so
                // this one goes in before the blit encoder is created.
                if let Some(sync) = sync {
                    cmd.encodeWaitForEvent_value(ProtocolObject::from_ref(sync.wgpu_done), sync.wait_value);
                }
                let Some(enc) = cmd.blitCommandEncoder() else { return };
                unsafe {
                    enc.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
                        src,
                        0, 0,
                        MTLOrigin { x: 0, y: 0, z: 0 },
                        MTLSize {
                            width:  frame.width  as usize,
                            height: frame.height as usize,
                            depth:  1,
                        },
                        dst,
                        0, 0,
                        MTLOrigin { x: 0, y: 0, z: 0 },
                    );
                }
                enc.endEncoding();
                if let Some(sync) = sync {
                    cmd.encodeSignalEvent_value(ProtocolObject::from_ref(sync.blit_done), sync.signal_value);
                }
                cmd.commit();
                ok = true;
            });
        });

        ok
    }

    /// Blit with both hazards fenced on the GPU, no CPU block.
    #[cfg(target_os = "macos")]
    fn blit_with_events(
        frame: &syphon_core::Frame,
        output: &wgpu::Texture,
        metal_queue: &ProtocolObject<dyn MTLCommandQueue>,
        sync: &mut EventSync,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> bool {
        // If last frame's signal is still sitting in the queue, wgpu never
        // submitted, so `wgpu_done` never advanced and never will for that
        // value. A command buffer waiting on it would park Syphon's queue on
        // the GPU with nothing able to release it — a hang strictly worse than
        // the CPU drain this replaces. Withdraw the stale signal, drain once,
        // and start the chain over from a value that is already satisfied.
        let stalled = syphon_metal::wgpu_interop::queue_take_pending_signal(queue, &sync.wgpu_done);
        if stalled {
            log::debug!(
                "[SyphonWgpuInput] wgpu has not submitted since the last frame; \
                 draining once and restarting the fence chain"
            );
            crate::drain_wgpu_before_blit(device, "receive_texture (resync)");
        }

        let n = sync.frame + 1;
        let blit_sync = BlitSync {
            wgpu_done: &sync.wgpu_done,
            // Shared events start at 0 and waits are `>=`, so the first frame
            // — and any resync — passes straight through.
            wait_value: if stalled { 0 } else { sync.frame },
            blit_done: &sync.blit_done,
            signal_value: n,
        };

        if !Self::gpu_blit(frame, output, metal_queue, Some(&blit_sync)) {
            return false;
        }
        sync.frame = n;

        // Hold wgpu's reads until the blit lands, and have its submit release
        // the *next* blit when those reads are done.
        if !syphon_metal::wgpu_interop::queue_wait_for_event(queue, &sync.blit_done, n)
            || !syphon_metal::wgpu_interop::queue_signal_event(queue, &sync.wgpu_done, n)
        {
            // The probe in `EventSync::new` passed, so this queue could carry
            // events a moment ago. Nothing to undo — the blit is committed and
            // correctly ordered against last frame; only this frame's
            // read-after-write gate is missing.
            log::warn!("[SyphonWgpuInput] could not stage frame {n} events on wgpu's queue");
        }
        true
    }

    pub fn server_name(&self) -> Option<&str> {
        self.connected_server.as_deref()
    }
}

impl Drop for SyphonWgpuInput {
    fn drop(&mut self) {
        self.disconnect();
    }
}
