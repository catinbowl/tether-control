use crate::host::macos::ffi::{
    AXIsProcessTrusted, AXIsProcessTrustedWithOptions, CFRelease, CFDictionaryCreate,
    kAXTrustedCheckOptionPrompt, kCFBooleanTrue,
};
use std::ptr;

/// Checks whether the current process has macOS Accessibility permissions (required for `CGEventTap`).
pub fn is_accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// Checks Accessibility permissions and triggers the native macOS System Settings prompt if not yet granted.
pub fn prompt_accessibility_permission() -> bool {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            ptr::null(),
            ptr::null(),
        );
        let trusted = AXIsProcessTrustedWithOptions(dict);
        if !dict.is_null() {
            CFRelease(dict);
        }
        trusted
    }
}

