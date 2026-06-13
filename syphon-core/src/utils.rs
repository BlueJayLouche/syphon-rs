//! Utility functions for Objective-C interop

use crate::{Result, SyphonError};
use std::ffi::{CStr, CString};

#[cfg(target_os = "macos")]
use objc::runtime::Object;

/// Convert a Rust string to an NSString
///
/// # Safety
/// The returned pointer must be released when done
#[cfg(target_os = "macos")]
pub fn to_nsstring(s: &str) -> Result<*mut Object> {
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};
    
    let c_string = CString::new(s).map_err(|e| {
        SyphonError::InvalidParameter(format!("Invalid string: {}", e))
    })?;
    
    unsafe {
        let cls = Class::get("NSString")
            .ok_or_else(|| SyphonError::FrameworkNotFound(
                "NSString class not found".to_string()
            ))?;
        
        let obj: *mut Object = msg_send![
            cls,
            stringWithUTF8String: c_string.as_ptr()
        ];
        
        if obj.is_null() {
            return Err(SyphonError::CreateFailed(
                "Failed to create NSString".to_string()
            ));
        }
        
        Ok(obj)
    }
}

/// Convert an NSString to a Rust String
// ponytail: caller must pass a valid NSString (or null) pointer — null is handled,
// garbage is UB. Kept non-`unsafe` to avoid churning every FFI call site.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[cfg(target_os = "macos")]
pub fn from_nsstring(obj: *mut Object) -> String {
    use objc::{msg_send, sel, sel_impl};
    
    unsafe {
        if obj.is_null() {
            return String::new();
        }
        
        let cstr: *const i8 = msg_send![obj, UTF8String];
        CStr::from_ptr(cstr)
            .to_string_lossy()
            .into_owned()
    }
}

/// Check if a class exists (for framework availability testing)
#[cfg(target_os = "macos")]
pub fn class_exists(name: &str) -> bool {
    use objc::runtime::Class;
    Class::get(name).is_some()
}
