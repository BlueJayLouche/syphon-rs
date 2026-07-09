//! End-to-end loopback acceptance test for the zero-copy pipeline.
//!
//! Single-process loopback: SyphonWgpuOutput publishes animated frames while
//! pumping the run loop (so directory announcements are delivered), then a
//! SyphonWgpuInput discovers, connects, GPU-blits frames into a wgpu texture,
//! and we read pixels back to verify real, changing image data arrived.
//! Exits non-zero (assert) on any failure, so it can be scripted:
//!
//! ```sh
//! cargo run -p syphon-examples --example verify_loopback
//! ```
//!
//! Note: both sides need the run loop pumped for directory announcements —
//! that's why the plain `wgpu_sender` + `metal_client` pair can't discover
//! each other from two bare CLI processes.

#[cfg(target_os = "macos")]
fn pump_runloop(seconds: f64) {
    use objc2_core_foundation::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, seconds, false);
    }
}

#[cfg(target_os = "macos")]
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
    let size = (w * h * 4) as u64;
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: None, size, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r.is_ok()); });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    assert!(rx.recv().unwrap(), "buffer map failed");
    let data = slice.get_mapped_range().to_vec();
    buf.unmap();
    data
}

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    env_logger::init();
    let (w, h) = (640u32, 360u32);
    let server_name = "objc2-verify-loopback";

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        .expect("adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("device");

    // Render target the "sender" draws into.
    let render_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("render"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let mut output = syphon_wgpu::SyphonWgpuOutput::new(server_name, &device, &queue, w, h)
        .expect("SyphonWgpuOutput");
    assert!(output.is_zero_copy(), "sender did not take the zero-copy path");
    println!("✓ sender created (zero-copy)");

    let mut input = syphon_wgpu::SyphonWgpuInput::new(&device, &queue);

    // Publish + pump until the directory sees our server, then connect by info.
    let mut connected = false;
    let mut frame_no = 0u32;
    let start = std::time::Instant::now();
    let mut received: Vec<(u8, u8, u8, u8)> = Vec::new(); // first-pixel BGRA per received frame

    while start.elapsed().as_secs() < 30 {
        frame_no += 1;
        // Draw an animated solid color (b, g, r ramp based on frame count).
        let color = wgpu::Color {
            r: (frame_no % 200) as f64 / 255.0,
            g: 0.5,
            b: 1.0 - (frame_no % 200) as f64 / 255.0,
            a: 1.0,
        };
        let view = render_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let _rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view, resolve_target: None, depth_slice: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(color), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None, timestamp_writes: None,
                occlusion_query_set: None, multiview_mask: None,
            });
        }
        queue.submit([enc.finish()]);

        let status = output.publish(&render_tex, &device, &queue);

        if !connected {
            let servers = syphon_core::SyphonServerDirectory::servers();
            if let Some(info) = servers.iter().find(|s| s.name == server_name) {
                println!("✓ server discovered via directory (app: {})", info.app_name);
                input.connect_by_info(info).expect("connect_by_info");
                connected = true;
                println!("✓ client connected");
            }
        } else {
            if input.receive_texture(&device, &queue) {
                let tex = input.output_texture().expect("output texture");
                let data = read_back(&device, &queue, tex, w, h);
                let px = (data[0], data[1], data[2], data[3]);
                println!("  received frame: first pixel BGRA = {:?}, publish status = {:?}", px, status);
                // Skip the warm-up frame: the first readback can race the very
                // first external blit in this synthetic immediate-readback loop.
                if px != (0, 0, 0, 0) { received.push(px); }
                if received.len() >= 5 { break; }
            }
        }

        pump_runloop(0.02);
    }

    assert!(connected, "FAIL: never discovered/connected to loopback server");
    assert!(received.len() >= 5, "FAIL: only received {} frames", received.len());
    // Frames must contain real data: alpha 255, green channel ~128 everywhere.
    for &(_b, g, _r, a) in &received {
        assert_eq!(a, 255, "alpha channel wrong — garbage frame?");
        assert!((g as i32 - 128).unsigned_abs() <= 2, "green channel {} != ~128 — garbage frame?", g);
    }
    // And the animated channels must actually change across frames (fresh frames, not one stale image).
    assert!(
        received.windows(2).any(|p| p[0].0 != p[1].0 || p[0].2 != p[1].2),
        "FAIL: pixel data identical across frames — stale/garbage transfer?"
    );

    println!("\n✓✓ VERIFIED: zero-copy sender → Syphon → zero-copy receiver, {} frames with correct, changing pixel data", received.len());
}
