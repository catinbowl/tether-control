use crate::coords::DisplayRect;
use crate::host::macos::ffi::{
    _dispatch_main_q, CGAssociateMouseAndMouseCursorPosition, CGDirectDisplayID, CGDisplayBounds,
    CGMainDisplayID, CGWarpMouseCursorPosition,
};
use anyhow::{Result, bail};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_foundation::{NSArray, NSString};
use tracing::{info, warn};

/// Configuration for registering a macOS `CGVirtualDisplay` matching the Linux client's monitor.
#[derive(Debug, Clone)]
pub struct VirtualDisplaySpec {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u16,
    /// Deterministic client identifier used as `serialNum` so macOS System Settings remembers
    /// the user's Display Arrangement across disconnects and restarts.
    pub serial_num: u32,
}

/// Owns a live CoreGraphics `CGVirtualDisplay` Objective-C object.
///
/// Dropping this struct releases the Objective-C object, immediately unregistering
/// the virtual external display from macOS WindowServer.
pub struct VirtualDisplay {
    _raw_display: Retained<AnyObject>,
    display_id: CGDirectDisplayID,
    spec: VirtualDisplaySpec,
}

impl VirtualDisplay {
    /// Creates and activates a macOS virtual external display using CoreGraphics' `CGVirtualDisplay` API.
    /// Must be called on the macOS main thread.
    pub fn create(spec: VirtualDisplaySpec) -> Result<Self> {
        if spec.width == 0 || spec.height == 0 {
            bail!("Virtual display dimensions must be non-zero");
        }

        unsafe {
            let desc_cls = AnyClass::get(c"CGVirtualDisplayDescriptor")
                .ok_or_else(|| anyhow::anyhow!("CGVirtualDisplayDescriptor class not found in CoreGraphics"))?;

            let mode_cls = AnyClass::get(c"CGVirtualDisplayMode")
                .ok_or_else(|| anyhow::anyhow!("CGVirtualDisplayMode class not found in CoreGraphics"))?;

            let settings_cls = AnyClass::get(c"CGVirtualDisplaySettings")
                .ok_or_else(|| anyhow::anyhow!("CGVirtualDisplaySettings class not found in CoreGraphics"))?;

            let disp_cls = AnyClass::get(c"CGVirtualDisplay")
                .ok_or_else(|| anyhow::anyhow!("CGVirtualDisplay class not found in CoreGraphics"))?;

            // 1. Allocate & configure CGVirtualDisplayDescriptor
            let desc_ptr: *mut AnyObject = msg_send![desc_cls, alloc];
            let desc_ptr: *mut AnyObject = msg_send![desc_ptr, init];
            let descriptor = Retained::from_raw(desc_ptr)
                .ok_or_else(|| anyhow::anyhow!("Failed to initialize CGVirtualDisplayDescriptor"))?;

            let ns_name = NSString::from_str(&spec.name);
            let main_queue: &AnyObject = &_dispatch_main_q;

            // Compute realistic physical dimensions (~96 PPI) so macOS WindowServer computes standard DPI.
            let width_mm = ((spec.width as f64) / 96.0) * 25.4;
            let height_mm = ((spec.height as f64) / 96.0) * 25.4;
            let phys_size = CGSize::new(width_mm.max(100.0), height_mm.max(60.0));

            let vendor_id: u32 = 0x5443; // "TC" (TetherControl)
            let product_id: u32 = (((spec.width & 0xFFFF) << 16) | (spec.height & 0xFFFF)).max(1);
            let serial_num: u32 = spec.serial_num.max(1);

            let _: () = msg_send![&*descriptor, setDispatchQueue: main_queue];
            let _: () = msg_send![&*descriptor, setName: &*ns_name];
            let _: () = msg_send![&*descriptor, setMaxPixelsWide: spec.width];
            let _: () = msg_send![&*descriptor, setMaxPixelsHigh: spec.height];
            let _: () = msg_send![&*descriptor, setSizeInMillimeters: phys_size];
            let _: () = msg_send![&*descriptor, setVendorID: vendor_id];
            let _: () = msg_send![&*descriptor, setProductID: product_id];
            let _: () = msg_send![&*descriptor, setSerialNum: serial_num];

            // Standard sRGB color primaries & D65 white point
            let _: () = msg_send![&*descriptor, setRedPrimary: CGPoint::new(0.6400, 0.3300)];
            let _: () = msg_send![&*descriptor, setGreenPrimary: CGPoint::new(0.3000, 0.6000)];
            let _: () = msg_send![&*descriptor, setBluePrimary: CGPoint::new(0.1500, 0.0600)];
            let _: () = msg_send![&*descriptor, setWhitePoint: CGPoint::new(0.3127, 0.3290)];

            // 2. Instantiate CGVirtualDisplay
            let disp_ptr: *mut AnyObject = msg_send![disp_cls, alloc];
            let disp_ptr: *mut AnyObject = msg_send![disp_ptr, initWithDescriptor: &*descriptor];
            let display = Retained::from_raw(disp_ptr)
                .ok_or_else(|| anyhow::anyhow!("CGVirtualDisplay initWithDescriptor: returned nil"))?;

            // 3. Configure CGVirtualDisplayMode & CGVirtualDisplaySettings (1:1 non-HiDPI matching client resolution)
            let refresh = (spec.refresh_hz.max(30)) as f64;
            let mode_ptr: *mut AnyObject = msg_send![mode_cls, alloc];
            let mode_ptr: *mut AnyObject = msg_send![
                mode_ptr,
                initWithWidth: spec.width,
                height: spec.height,
                refreshRate: refresh
            ];
            let mode = Retained::from_raw(mode_ptr)
                .ok_or_else(|| anyhow::anyhow!("Failed to create CGVirtualDisplayMode"))?;

            let settings_ptr: *mut AnyObject = msg_send![settings_cls, alloc];
            let settings_ptr: *mut AnyObject = msg_send![settings_ptr, init];
            let settings = Retained::from_raw(settings_ptr)
                .ok_or_else(|| anyhow::anyhow!("Failed to create CGVirtualDisplaySettings"))?;

            let modes_array = NSArray::from_retained_slice(&[mode]);
            let _: () = msg_send![&*settings, setHiDPI: 0_u32];
            let _: () = msg_send![&*settings, setModes: &*modes_array];

            let applied: bool = msg_send![&*display, applySettings: &*settings];
            if !applied {
                warn!("CGVirtualDisplay applySettings: returned false; display may use descriptor defaults");
            }

            let display_id: u32 = msg_send![&*display, displayID];
            if display_id == 0 {
                bail!("CGVirtualDisplay returned invalid displayID 0");
            }

            let bounds = query_display_bounds(display_id);
            info!(
                "Registered macOS Virtual External Display '{}' (ID={}): {}x{} @ {}Hz, bounds=({:.0}, {:.0}, {:.0}x{:.0})",
                spec.name,
                display_id,
                spec.width,
                spec.height,
                spec.refresh_hz,
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height
            );

            Ok(Self {
                _raw_display: display,
                display_id,
                spec,
            })
        }
    }

    pub fn display_id(&self) -> CGDirectDisplayID {
        self.display_id
    }

    pub fn spec(&self) -> &VirtualDisplaySpec {
        &self.spec
    }

    pub fn bounds(&self) -> DisplayRect {
        query_display_bounds(self.display_id)
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        info!(
            "Unregistering macOS Virtual External Display '{}' (ID={})",
            self.spec.name, self.display_id
        );
    }
}

pub fn query_display_bounds(display_id: CGDirectDisplayID) -> DisplayRect {
    let rect = unsafe { CGDisplayBounds(display_id) };
    DisplayRect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

/// Warps the macOS cursor back to the center of the MacBook's primary display.
pub fn warp_cursor_to_main_display() {
    unsafe {
        let main_id = CGMainDisplayID();
        let rect = CGDisplayBounds(main_id);
        let center = CGPoint::new(
            rect.origin.x + rect.size.width * 0.5,
            rect.origin.y + rect.size.height * 0.5,
        );
        let _ = CGWarpMouseCursorPosition(center);
        let _ = CGAssociateMouseAndMouseCursorPosition(true);
    }
}
