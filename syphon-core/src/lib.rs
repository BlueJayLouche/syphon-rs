//! # Syphon Core
//! 
//! Core Objective-C bindings for the Syphon framework on macOS.
//! 
//! ## Overview
//! 
//! Syphon is a macOS technology for sharing video frames between applications
//! with zero-copy GPU efficiency. This crate provides safe Rust bindings to
//! the Syphon Objective-C framework.
//! 
//! ## Usage
//!
//! Publishing frames lives in the `syphon-metal` / `syphon-wgpu` crates; this
//! crate handles server lifecycle, discovery, and receiving.
//!
//! ```no_run
//! use syphon_core::{SyphonServer, SyphonClient, SyphonServerDirectory};
//!
//! # fn main() -> syphon_core::Result<()> {
//! // Advertise a server.
//! let server = SyphonServer::new("My App", 1920, 1080)?;
//! println!("Serving {} at {:?}", server.name(), server.dimensions());
//!
//! // List available servers.
//! for info in SyphonServerDirectory::servers() {
//!     println!("Found: {}", info.display_name());
//! }
//!
//! // Connect to a server and grab a frame.
//! let client = SyphonClient::connect("Resolume Arena")?;
//! if let Some(_frame) = client.try_receive()? {
//!     // Use the frame...
//! }
//! # Ok(())
//! # }
//! ```

// syphon-core - Objective-C bindings for Syphon

#[cfg(target_os = "macos")]
mod iosurface_ext;
mod error;
mod server;
mod client;
mod directory;
mod utils;

pub use error::{SyphonError, Result};
pub use server::{SyphonServer, ServerOptions};
pub use client::{SyphonClient, Frame};
pub use directory::{SyphonServerDirectory, ServerInfo};
pub use utils::{to_nsstring, from_nsstring, class_exists};

/// Check if Syphon is available on this system
pub fn is_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        // Check if the SyphonServer class exists
        class_exists("SyphonServer")
    }
    
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Get version information about the Syphon framework
pub fn version() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc::runtime::Class;
        use objc::{msg_send, sel, sel_impl};
        
        unsafe {
            let cls = Class::get("SyphonServer")?;
            let version: *mut objc::runtime::Object = msg_send![cls, version];
            Some(utils::from_nsstring(version))
        }
    }
    
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_availability() {
        // On macOS, this depends on Syphon being installed
        // On other platforms, should always be false
        let available = is_available();
        
        #[cfg(target_os = "macos")]
        println!("Syphon available: {}", available);
        
        #[cfg(not(target_os = "macos"))]
        assert!(!available);
    }

    #[test]
    fn test_version() {
        if let Some(v) = version() {
            println!("Syphon version: {}", v);
        }
    }
}
