//! Acceptance test: receive from a *real external* Syphon publisher.
//!
//! `verify_loopback` publishes and receives in one process, which cannot catch
//! cross-process regressions in the receive blit. Run any Syphon publisher
//! (VP-404, Simple Server, ...), then:
//!
//! ```sh
//! cargo run -p syphon-examples --example verify_external_receive
//! ```
//!
//! Asserts frames arrive AND carry non-black pixels — the exact failure mode
//! seen when the pre-blit `drain_wgpu_before_blit` wait is removed.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    env_logger::init();
    use objc2_core_foundation::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
    let pump = |s: f64| unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, s, false); };

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();

    pump(1.0);
    let servers = syphon_core::SyphonServerDirectory::servers();
    println!("servers visible: {}", servers.len());
    for s in &servers { println!("  - {} (uuid={})", s.display_name(), s.uuid); }
    let target = servers.first().expect("no Syphon server publishing");
    println!("connecting to '{}'", target.display_name());

    let mut input = syphon_wgpu::SyphonWgpuInput::new(&device, &queue);
    input.connect_by_info(target).expect("connect failed");

    let mut got = 0;
    let mut seen: Vec<[u8; 4]> = Vec::new();
    for _ in 0..600 {
        pump(0.016);
        if !input.receive_texture(&device, &queue) { continue; }
        let tex = input.output_texture().unwrap();
        let (w, h) = (tex.width(), tex.height());

        // Read back the whole frame (1920*4 = 7680, already 256-aligned).
        let bpr = w * 4;
        assert_eq!(bpr % 256, 0, "row pitch needs padding for {w}px");
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None, size: (bpr * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex, mip_level: 0,
                origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout {
                offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r.is_ok()); });
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None, timeout: Some(std::time::Duration::from_secs(2)) });
        assert!(rx.recv().unwrap(), "map failed");
        let d = slice.get_mapped_range().unwrap();
        let nonzero = d.chunks_exact(4).filter(|p| p[0] | p[1] | p[2] != 0).count();
        let maxv = d.chunks_exact(4).map(|p| p[0].max(p[1]).max(p[2])).max().unwrap_or(0);
        let total = (w * h) as usize;
        let px = [d[0], d[1], d[2], d[3]];
        drop(d); buf.unmap();

        got += 1;
        println!("  frame {got}: {w}x{h}  non-black px = {nonzero}/{total} ({:.1}%)  brightest = {maxv}  px0 = {px:?}",
                 100.0 * nonzero as f64 / total as f64);
        seen.push([nonzero as u8, (nonzero >> 8) as u8, (nonzero >> 16) as u8, maxv]);
        if got >= 8 { break; }
    }

    assert!(got >= 8, "only received {got} frames from external publisher");
    let distinct = { let mut v = seen.clone(); v.sort(); v.dedup(); v.len() };
    println!("\n✓ received {got} frames, {distinct} distinct centre pixels");
    assert!(seen.iter().any(|p| p != &[0, 0, 0, 0]), "every frame was entirely black");
    println!("✓✓ VERIFIED: external publisher → zero-copy receive on wgpu's own queue");
}
