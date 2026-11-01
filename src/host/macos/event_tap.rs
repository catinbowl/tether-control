#![allow(non_upper_case_globals)]

use crate::coords::{ClientResolution, DisplayRect, map_pointer_to_client};
use crate::keymap::{KEY_CAPSLOCK, macos_keycode_to_evdev, modifier_is_pressed};
use crate::host::macos::ffi::*;
use crate::host::macos::virtual_display::{query_display_bounds, warp_cursor_to_main_display};
use crate::protocol::{HostMessage, MouseBtn};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Fast outbound queue between the real-time `CGEventTap` callback and the TCP writer thread.
/// Coalesces consecutive `MouseMove` packets under network backpressure while preserving
/// exact ordering of all button, scroll, keyboard, and lifecycle events.
pub struct OutboundQueue {
    queue: Mutex<VecDeque<HostMessage>>,
    cvar: Condvar,
}

impl Default for OutboundQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OutboundQueue {
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(64)),
            cvar: Condvar::new(),
        }
    }

    #[inline]
    pub fn push(&self, msg: HostMessage) {
        if let Ok(mut q) = self.queue.lock() {
            if let HostMessage::MouseMove { x, y } = msg
                && let Some(HostMessage::MouseMove {
                    x: prev_x,
                    y: prev_y,
                }) = q.back_mut()
            {
                *prev_x = x;
                *prev_y = y;
                self.cvar.notify_one();
                return;
            }
            if q.len() > 2048 {
                // Drop oldest queued move if disconnected/stalled
                let _ = q.pop_front();
            }
            q.push_back(msg);
            self.cvar.notify_one();
        }
    }

    pub fn clear(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.clear();
        }
    }

    /// Waits up to `timeout` for at least one message and drains all available messages into `out`.
    pub fn drain_wait(&self, out: &mut Vec<HostMessage>, timeout: Duration) {
        out.clear();
        let Ok(mut q) = self.queue.lock() else {
            return;
        };
        if q.is_empty() {
            let Ok((guard, _)) = self.cvar.wait_timeout(q, timeout) else {
                return;
            };
            q = guard;
        }
        out.extend(q.drain(..));
    }
}

/// Lock-free atomic state shared between the Main Thread (`CGVirtualDisplay` + Menu Bar UI),
/// the `CGEventTap` real-time thread, and the TCP session manager.
pub struct SharedTapState {
    active_display_id: AtomicU32,
    bounds_x: AtomicU64,
    bounds_y: AtomicU64,
    bounds_w: AtomicU64,
    bounds_h: AtomicU64,
    client_w: AtomicU32,
    client_h: AtomicU32,
    pub swap_cmd_ctrl: AtomicBool,
    pub pointer_on_client: AtomicBool,
    pub tap_installed: AtomicBool,
    pub outbound: OutboundQueue,
}

impl SharedTapState {
    pub fn new(swap_cmd_ctrl: bool) -> Arc<Self> {
        Arc::new(Self {
            active_display_id: AtomicU32::new(0),
            bounds_x: AtomicU64::new(0.0_f64.to_bits()),
            bounds_y: AtomicU64::new(0.0_f64.to_bits()),
            bounds_w: AtomicU64::new(0.0_f64.to_bits()),
            bounds_h: AtomicU64::new(0.0_f64.to_bits()),
            client_w: AtomicU32::new(0),
            client_h: AtomicU32::new(0),
            swap_cmd_ctrl: AtomicBool::new(swap_cmd_ctrl),
            pointer_on_client: AtomicBool::new(false),
            tap_installed: AtomicBool::new(false),
            outbound: OutboundQueue::new(),
        })
    }

    pub fn set_active_virtual_display(
        &self,
        display_id: CGDirectDisplayID,
        client_width: u32,
        client_height: u32,
    ) {
        self.client_w.store(client_width, Ordering::Release);
        self.client_h.store(client_height, Ordering::Release);
        self.store_bounds(query_display_bounds(display_id));
        self.active_display_id.store(display_id, Ordering::Release);
    }

    pub fn clear_virtual_display(&self) {
        let was_active = self.pointer_on_client.swap(false, Ordering::AcqRel);
        self.active_display_id.store(0, Ordering::Release);
        self.store_bounds(DisplayRect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        });
        if was_active {
            warp_cursor_to_main_display();
        }
    }

    pub fn refresh_active_bounds(&self) {
        let id = self.active_display_id.load(Ordering::Acquire);
        if id != 0 {
            let bounds = query_display_bounds(id);
            self.store_bounds(bounds);
            debug!(
                "Updated virtual display #{} bounds: ({:.0}, {:.0}, {:.0}x{:.0})",
                id, bounds.x, bounds.y, bounds.width, bounds.height
            );
        }
    }

    pub fn current_bounds(&self) -> Option<(CGDirectDisplayID, DisplayRect)> {
        let id = self.active_display_id.load(Ordering::Acquire);
        if id == 0 {
            return None;
        }
        Some((id, self.load_bounds()))
    }

    #[inline]
    fn store_bounds(&self, rect: DisplayRect) {
        self.bounds_x.store(rect.x.to_bits(), Ordering::Release);
        self.bounds_y.store(rect.y.to_bits(), Ordering::Release);
        self.bounds_w.store(rect.width.to_bits(), Ordering::Release);
        self.bounds_h.store(rect.height.to_bits(), Ordering::Release);
    }

    #[inline]
    fn load_bounds(&self) -> DisplayRect {
        DisplayRect {
            x: f64::from_bits(self.bounds_x.load(Ordering::Relaxed)),
            y: f64::from_bits(self.bounds_y.load(Ordering::Relaxed)),
            width: f64::from_bits(self.bounds_w.load(Ordering::Relaxed)),
            height: f64::from_bits(self.bounds_h.load(Ordering::Relaxed)),
        }
    }

    #[inline]
    fn load_client_resolution(&self) -> ClientResolution {
        ClientResolution {
            width: self.client_w.load(Ordering::Relaxed),
            height: self.client_h.load(Ordering::Relaxed),
        }
    }
}

struct TapCallbackContext {
    tap_port: CFMachPortRef,
    shared: Arc<SharedTapState>,
    was_on_client: bool,
    mac_drag_buttons_mask: u32,
}

unsafe extern "C" fn display_reconfig_callback(
    _display: CGDirectDisplayID,
    _flags: CGDisplayChangeSummaryFlags,
    user_info: *mut c_void,
) {
    if user_info.is_null() {
        return;
    }
    let shared = unsafe { &*(user_info as *const SharedTapState) };
    shared.refresh_active_bounds();
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    type_: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if user_info.is_null() || event.is_null() {
        return event;
    }
    let ctx = unsafe { &mut *(user_info as *mut TapCallbackContext) };

    if type_ == kCGEventTapDisabledByTimeout || type_ == kCGEventTapDisabledByUserInput {
        warn!("macOS disabled CGEventTap (reason=0x{:x}); re-enabling immediately", type_);
        if !ctx.tap_port.is_null() {
            unsafe { CGEventTapEnable(ctx.tap_port, true) };
        }
        return event;
    }

    let display_id = ctx.shared.active_display_id.load(Ordering::Acquire);
    if display_id == 0 {
        if ctx.was_on_client {
            ctx.was_on_client = false;
            ctx.shared.pointer_on_client.store(false, Ordering::Release);
            ctx.shared.outbound.push(HostMessage::PointerLeave);
        }
        ctx.mac_drag_buttons_mask = 0;
        return event;
    }

    let is_keyboard_event = matches!(
        type_,
        kCGEventKeyDown | kCGEventKeyUp | kCGEventFlagsChanged
    );

    if is_keyboard_event {
        if !ctx.was_on_client {
            return event;
        }
        return unsafe { handle_keyboard_event(ctx, type_, event) };
    }

    // Mouse / Scroll events: evaluate cursor location against virtual external display bounds
    let loc = unsafe { CGEventGetLocation(event) };
    let bounds = ctx.shared.load_bounds();
    let client_res = ctx.shared.load_client_resolution();
    let mapped = map_pointer_to_client(bounds, client_res, loc.x, loc.y);

    // Track mouse drags initiated on the MacBook screen so window drags near the border don't get stuck
    if mapped.is_none() && !ctx.was_on_client {
        update_mac_button_mask(&mut ctx.mac_drag_buttons_mask, type_, event);
    }
    if ctx.mac_drag_buttons_mask != 0 {
        update_mac_button_mask(&mut ctx.mac_drag_buttons_mask, type_, event);
        return event;
    }

    match mapped {
        None => {
            if ctx.was_on_client {
                ctx.was_on_client = false;
                ctx.shared.pointer_on_client.store(false, Ordering::Release);
                ctx.shared.outbound.push(HostMessage::PointerLeave);
            }
            event
        }
        Some((cx, cy)) => {
            if !ctx.was_on_client {
                ctx.was_on_client = true;
                ctx.shared.pointer_on_client.store(true, Ordering::Release);
                ctx.shared
                    .outbound
                    .push(HostMessage::PointerEnter { x: cx, y: cy });
            } else {
                ctx.shared
                    .outbound
                    .push(HostMessage::MouseMove { x: cx, y: cy });
            }

            match type_ {
                kCGEventMouseMoved => event,
                kCGEventLeftMouseDragged
                | kCGEventRightMouseDragged
                | kCGEventOtherMouseDragged => {
                    // Convert drag to plain cursor move locally so macOS moves the cursor smoothly
                    // without dragging Finder desktop selection or UI widgets on the virtual display.
                    unsafe { CGEventSetType(event, kCGEventMouseMoved) };
                    event
                }
                kCGEventLeftMouseDown | kCGEventLeftMouseUp => {
                    ctx.shared.outbound.push(HostMessage::MouseButton {
                        button: MouseBtn::Left,
                        pressed: type_ == kCGEventLeftMouseDown,
                    });
                    ptr::null_mut()
                }
                kCGEventRightMouseDown | kCGEventRightMouseUp => {
                    ctx.shared.outbound.push(HostMessage::MouseButton {
                        button: MouseBtn::Right,
                        pressed: type_ == kCGEventRightMouseDown,
                    });
                    ptr::null_mut()
                }
                kCGEventOtherMouseDown | kCGEventOtherMouseUp => {
                    let btn_num =
                        unsafe { CGEventGetIntegerValueField(event, kCGMouseEventButtonNumber) };
                    ctx.shared.outbound.push(HostMessage::MouseButton {
                        button: MouseBtn::from_macos_button_number(btn_num),
                        pressed: type_ == kCGEventOtherMouseDown,
                    });
                    ptr::null_mut()
                }
                kCGEventScrollWheel => {
                    let is_continuous = unsafe {
                        CGEventGetIntegerValueField(event, kCGScrollWheelEventIsContinuous) != 0
                    };
                    let (v_hi_res, h_hi_res) = if is_continuous {
                        let py = unsafe {
                            CGEventGetDoubleValueField(event, kCGScrollWheelEventPointDeltaAxis1)
                        };
                        let px = unsafe {
                            CGEventGetDoubleValueField(event, kCGScrollWheelEventPointDeltaAxis2)
                        };
                        ((py * 8.0).round() as i32, (-px * 8.0).round() as i32)
                    } else {
                        let dy = unsafe {
                            CGEventGetIntegerValueField(event, kCGScrollWheelEventDeltaAxis1) as i32
                        };
                        let dx = unsafe {
                            CGEventGetIntegerValueField(event, kCGScrollWheelEventDeltaAxis2) as i32
                        };
                        (dy * 120, -dx * 120)
                    };
                    if v_hi_res != 0 || h_hi_res != 0 {
                        ctx.shared
                            .outbound
                            .push(HostMessage::Scroll { v_hi_res, h_hi_res });
                    }
                    ptr::null_mut()
                }
                _ => event,
            }
        }
    }
}

fn update_mac_button_mask(mask: &mut u32, type_: CGEventType, event: CGEventRef) {
    match type_ {
        kCGEventLeftMouseDown => *mask |= 1 << 0,
        kCGEventLeftMouseUp => *mask &= !(1 << 0),
        kCGEventRightMouseDown => *mask |= 1 << 1,
        kCGEventRightMouseUp => *mask &= !(1 << 1),
        kCGEventOtherMouseDown => {
            let b = (unsafe { CGEventGetIntegerValueField(event, kCGMouseEventButtonNumber) }
                .clamp(2, 30)) as u32;
            *mask |= 1 << b;
        }
        kCGEventOtherMouseUp => {
            let b = (unsafe { CGEventGetIntegerValueField(event, kCGMouseEventButtonNumber) }
                .clamp(2, 30)) as u32;
            *mask &= !(1 << b);
        }
        _ => {}
    }
}

unsafe fn handle_keyboard_event(
    ctx: &mut TapCallbackContext,
    type_: CGEventType,
    event: CGEventRef,
) -> CGEventRef {
    let vk = unsafe { CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) } as u16;
    let flags = unsafe { CGEventGetFlags(event) };

    // Emergency escape hatch: Ctrl + Option + Command + Escape immediately returns cursor to MacBook
    let escape_mods =
        kCGEventFlagMaskControl | kCGEventFlagMaskAlternate | kCGEventFlagMaskCommand;
    if type_ == kCGEventKeyDown && vk == 0x35 && (flags & escape_mods) == escape_mods {
        info!("Emergency escape hotkey (Ctrl+Opt+Cmd+Esc) triggered; returning cursor to MacBook");
        ctx.was_on_client = false;
        ctx.shared.pointer_on_client.store(false, Ordering::Release);
        ctx.shared.outbound.push(HostMessage::PointerLeave);
        warp_cursor_to_main_display();
        return ptr::null_mut();
    }

    let swap_cmd_ctrl = ctx.shared.swap_cmd_ctrl.load(Ordering::Relaxed);

    match type_ {
        kCGEventKeyDown | kCGEventKeyUp => {
            let autorepeat =
                unsafe { CGEventGetIntegerValueField(event, kCGKeyboardEventAutorepeat) != 0 };
            if autorepeat && type_ == kCGEventKeyDown {
                // Linux kernel EV_REP handles held-key autorepeat natively without LAN jitter
                return ptr::null_mut();
            }
            if let Some(evdev_code) = macos_keycode_to_evdev(vk, swap_cmd_ctrl) {
                ctx.shared.outbound.push(HostMessage::Key {
                    evdev_code,
                    pressed: type_ == kCGEventKeyDown,
                });
            }
            ptr::null_mut()
        }
        kCGEventFlagsChanged => {
            if vk == 0x39 {
                // macOS CapsLock sends a single FlagsChanged per toggle; Linux toggles on press+release
                ctx.shared.outbound.push(HostMessage::Key {
                    evdev_code: KEY_CAPSLOCK,
                    pressed: true,
                });
                ctx.shared.outbound.push(HostMessage::Key {
                    evdev_code: KEY_CAPSLOCK,
                    pressed: false,
                });
                return ptr::null_mut();
            }
            if let (Some(evdev_code), Some(pressed)) = (
                macos_keycode_to_evdev(vk, swap_cmd_ctrl),
                modifier_is_pressed(vk, flags),
            ) {
                ctx.shared.outbound.push(HostMessage::Key {
                    evdev_code,
                    pressed,
                });
            }
            ptr::null_mut()
        }
        _ => event,
    }
}

/// Spawns the dedicated real-time `CFRunLoop` thread hosting the `CGEventTap` and display reconfiguration listener.
/// Automatically retries `CGEventTapCreate` if Accessibility permissions are granted while the app is running.
pub fn spawn_event_tap_thread(shared: Arc<SharedTapState>) {
    // Register display reconfiguration callback so macOS Display Arrangement edits take effect live.
    let shared_ptr = Arc::into_raw(Arc::clone(&shared)) as *mut c_void;
    unsafe {
        let rc = CGDisplayRegisterReconfigurationCallback(display_reconfig_callback, shared_ptr);
        if rc != 0 {
            warn!("CGDisplayRegisterReconfigurationCallback returned {}", rc);
        }
    }

    thread::Builder::new()
        .name("tether-event-tap".into())
        .spawn(move || {
            let mask: CGEventMask = cg_event_mask_bit(kCGEventMouseMoved)
                | cg_event_mask_bit(kCGEventLeftMouseDown)
                | cg_event_mask_bit(kCGEventLeftMouseUp)
                | cg_event_mask_bit(kCGEventLeftMouseDragged)
                | cg_event_mask_bit(kCGEventRightMouseDown)
                | cg_event_mask_bit(kCGEventRightMouseUp)
                | cg_event_mask_bit(kCGEventRightMouseDragged)
                | cg_event_mask_bit(kCGEventOtherMouseDown)
                | cg_event_mask_bit(kCGEventOtherMouseUp)
                | cg_event_mask_bit(kCGEventOtherMouseDragged)
                | cg_event_mask_bit(kCGEventScrollWheel)
                | cg_event_mask_bit(kCGEventKeyDown)
                | cg_event_mask_bit(kCGEventKeyUp)
                | cg_event_mask_bit(kCGEventFlagsChanged);

            let mut ctx = Box::new(TapCallbackContext {
                tap_port: ptr::null_mut(),
                shared: Arc::clone(&shared),
                was_on_client: false,
                mac_drag_buttons_mask: 0,
            });
            let ctx_ptr: *mut c_void = (&mut *ctx as *mut TapCallbackContext).cast();

            let tap_port = loop {
                let port = unsafe {
                    CGEventTapCreate(
                        kCGSessionEventTap,
                        kCGHeadInsertEventTap,
                        kCGEventTapOptionDefault,
                        mask,
                        event_tap_callback,
                        ctx_ptr,
                    )
                };
                if !port.is_null() {
                    break port;
                }
                shared.tap_installed.store(false, Ordering::Release);
                thread::sleep(Duration::from_secs(2));
            };

            ctx.tap_port = tap_port;

            unsafe {
                let source = CFMachPortCreateRunLoopSource(ptr::null(), tap_port, 0);
                if source.is_null() {
                    warn!("CFMachPortCreateRunLoopSource returned NULL");
                    return;
                }
                let rl = CFRunLoopGetCurrent();
                CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap_port, true);
                shared.tap_installed.store(true, Ordering::Release);
                info!("Installed macOS CGEventTap on dedicated real-time CFRunLoop");
                CFRunLoopRun();
                CFRelease(source);
                CFRelease(tap_port);
            }
        })
        .expect("Failed spawning CGEventTap thread");
}
