//! WGPU Syphon Sender - Zero-copy GPU-to-GPU test
//!
//! This example creates a wgpu context, renders an animated test pattern,
//! and publishes it via Syphon using zero-copy IOSurface sharing.

use std::time::Instant;

fn main() {
    env_logger::init();
    
    println!("=== WGPU Syphon Sender (Zero-Copy) ===\n");
    
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("This example requires macOS");
        std::process::exit(1);
    }
    
    #[cfg(target_os = "macos")]
    run();
}

#[cfg(target_os = "macos")]
fn run() {
    // Check if Syphon is available
    if !syphon_wgpu::is_available() {
        eprintln!("Error: Syphon is not available");
        std::process::exit(1);
    }
    println!("✓ Syphon is available!");
    
    // Configuration
    let width = 1280u32;
    let height = 720u32;
    let server_name = "WGPU Zero-Copy Test";
    
    // Create wgpu instance
    println!("\nInitializing wgpu...");
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    
    // Create wgpu adapter and device - FORCE high performance (discrete GPU)
    println!("Requesting high-performance adapter...");
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    })).expect("Failed to create adapter");
    
    let adapter_info = adapter.get_info();
    println!("✓ Adapter: {} ({:?})", adapter_info.name, adapter_info.device_type);
    
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Syphon Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        },
    )).expect("Failed to create device");
    
    println!("✓ wgpu device created");
    
    // Create the output texture that we'll render to
    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Output Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    
    // Create Syphon output (zero-copy IOSurface-based)
    println!("\nCreating Syphon output '{}' ({}x{})...", server_name, width, height);
    let mut syphon_output = syphon_wgpu::SyphonWgpuOutput::new(
        server_name,
        &device,
        &queue,
        width,
        height
    ).expect("Failed to create Syphon output");
    println!("✓ Syphon output created (zero-copy IOSurface)");
    
    println!("\n🎬 Broadcasting to Syphon...");
    println!("Open any Syphon client to view.");
    println!("Press Ctrl+C to exit.\n");
    
    let start_time = Instant::now();
    let mut frame_count = 0u64;
    let mut last_fps_print = Instant::now();
    
    loop {
        let elapsed = start_time.elapsed().as_secs_f32();
        
        // Render a frame (animated color)
        let output_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: (elapsed.sin() * 0.5 + 0.5) as f64,
                            g: ((elapsed + 2.0).sin() * 0.5 + 0.5) as f64,
                            b: ((elapsed + 4.0).sin() * 0.5 + 0.5) as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        
        queue.submit(std::iter::once(encoder.finish()));
        
        // Publish to Syphon — check status to confirm zero-copy is active.
        let status = syphon_output.publish(&output_texture, &device, &queue);
        if matches!(status, syphon_wgpu::PublishStatus::CpuFallback) {
            log::warn!("Syphon is using CPU fallback — check Metal interop setup");
        }
        
        frame_count += 1;
        
        // Print stats every second
        if last_fps_print.elapsed().as_secs() >= 1 {
            let fps = frame_count as f64 / start_time.elapsed().as_secs_f64();
            let clients = syphon_output.client_count();
            
            print!("\r📡 {} clients | {} frames | {:.1} FPS    ", 
                clients, frame_count, fps);
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
            
            last_fps_print = Instant::now();
        }
        
        // ~60 FPS target. Spend the wait *in the CFRunLoop* rather than
        // asleep: Syphon announces a server over distributed notifications,
        // and without a run loop this process never publishes into the
        // directory, so no other process can discover it.
        unsafe {
            use objc2_core_foundation::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.016, false);
        }
    }
}
