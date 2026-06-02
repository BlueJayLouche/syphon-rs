//! Direct Metal Syphon Client Example - Zero-Copy Edition
//!
//! This example demonstrates how to receive frames from a Syphon server
//! and access them as Metal textures without any CPU copies.
//!
//! Usage:
//!   cargo run --example metal_client -- "Server Name"
//!
//! If no server name is provided, it will list available servers.

use std::time::{Duration, Instant};

fn main() {
    // Initialize logging
    env_logger::init();

    println!("=== Syphon Direct Metal Client Example (Zero-Copy) ===\n");

    // Check if Syphon is available
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Error: This example requires macOS.");
        std::process::exit(1);
    }

    #[cfg(target_os = "macos")]
    {
        if !syphon_core::is_available() {
            eprintln!("Error: Syphon framework is not available.");
            eprintln!("Make sure Syphon.framework is installed.");
            std::process::exit(1);
        }

        println!("✓ Syphon framework is available");

        // Initialize Metal
        let metal_ctx = match syphon_metal::MetalContext::system_default() {
            Some(ctx) => {
                println!("✓ Metal context created");
                ctx
            }
            None => {
                eprintln!("✗ Metal is not available on this system");
                std::process::exit(1);
            }
        };

        // Get server name from command line or discover servers
        let server_name = match std::env::args().nth(1) {
            Some(name) => name,
            None => {
                println!("\nNo server name provided. Discovering available servers...\n");

                let servers = syphon_core::SyphonServerDirectory::servers();

                if servers.is_empty() {
                    eprintln!("No Syphon servers found.");
                    eprintln!("Make sure a Syphon server is running.");
                    std::process::exit(1);
                }

                println!("Found {} server(s):", servers.len());
                for (i, server) in servers.iter().enumerate() {
                    println!(
                        "  {}. {} (app: {})",
                        i + 1,
                        if server.name.is_empty() {
                            "<empty>"
                        } else {
                            &server.name
                        },
                        server.app_name
                    );
                }

                println!("\nUsage: cargo run --example metal_client -- \"Server Name\"");
                std::process::exit(0);
            }
        };

        println!("\nConnecting to server: '{}'...", server_name);

        // Create and connect client
        let client = match syphon_core::SyphonClient::connect(&server_name) {
            Ok(c) => {
                println!("✓ Connected to server");
                println!("  Server name: {}", c.server_name());
                println!("  App name: {}", c.server_app());
                c
            }
            Err(e) => {
                eprintln!("✗ Failed to connect: {}", e);
                std::process::exit(1);
            }
        };

        println!("\nReceiving frames with zero-copy Metal interop...");
        println!("Press Ctrl+C to exit\n");

        let start_time = Instant::now();
        let mut frame_count = 0u64;
        let mut last_fps_time = Instant::now();
        let mut texture_creation_times = Vec::new();

        // Main loop
        loop {
            match client.try_receive() {
                Ok(Some(frame)) => {
                    frame_count += 1;

                    // ZERO-COPY: Create Metal texture directly from IOSurface
                    let tex_start = Instant::now();
                    
                    let surface = frame.iosurface();
                    let texture = metal_ctx.create_texture_from_iosurface(
                        surface,
                        frame.width,
                        frame.height,
                    );
                    
                    let tex_elapsed = tex_start.elapsed();
                    texture_creation_times.push(tex_elapsed);

                    match texture {
                        Some(tex) => {
                            // Successfully created Metal texture
                            // In a real app, you would use this texture in your render pipeline
                            let info = format!(
                                "Frame {}: {}x{} (texture: {}x{} px, format: {:?})",
                                frame_count,
                                frame.width,
                                frame.height,
                                tex.width(),
                                tex.height(),
                                tex.pixel_format()
                            );

                            // Print every 30th frame to avoid spam
                            if frame_count % 30 == 0 {
                                println!("{}", info);
                            }

                            // Texture is automatically released here
                        }
                        None => {
                            eprintln!(
                                "Frame {}: Failed to create Metal texture",
                                frame_count
                            );
                        }
                    }

                    // Frame (and its IOSurface) is released here, returned to Syphon pool
                }
                Ok(None) => {
                    // No new frame, sleep briefly
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => {
                    eprintln!("Error receiving frame: {}", e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }

            // Print FPS every 5 seconds
            if last_fps_time.elapsed() >= Duration::from_secs(5) {
                let elapsed = start_time.elapsed().as_secs_f32();
                let fps = frame_count as f32 / elapsed;
                
                // Calculate average texture creation time
                let avg_tex_time = if !texture_creation_times.is_empty() {
                    let total: u128 = texture_creation_times.iter().map(|d| d.as_micros()).sum();
                    total as f64 / texture_creation_times.len() as f64
                } else {
                    0.0
                };

                println!(
                    "\n[Stats] FPS: {:.1}, Total frames: {}, Avg texture creation: {:.1}µs\n",
                    fps, frame_count, avg_tex_time
                );
                
                texture_creation_times.clear();
                last_fps_time = Instant::now();
            }

            // Exit after 60 seconds for demo purposes
            if start_time.elapsed() >= Duration::from_secs(60) {
                println!("\nDemo complete (60 seconds elapsed).");
                break;
            }
        }

        // Summary
        let elapsed = start_time.elapsed().as_secs_f32();
        println!("\n=== Summary ===");
        println!("Total frames: {}", frame_count);
        println!("Average FPS: {:.1}", frame_count as f32 / elapsed);
        println!("Duration: {:.1}s", elapsed);
        println!("\n✓ Zero-copy Metal client test complete");
        println!("  No CPU copies were made - all frames accessed via IOSurface directly");

        // Cleanup
        client.stop();
    }
}
