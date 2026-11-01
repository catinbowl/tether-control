use crate::keymap::{
    BTN_EXTRA, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_SIDE, MAX_KEYBOARD_KEYCODE,
};
use crate::protocol::MouseBtn;
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::thread;
use std::time::Duration;
use tracing::{debug, info, warn};

const UINPUT_MAX_NAME_SIZE: usize = 80;
const BUS_USB: u16 = 0x03;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const EV_REP: u16 = 0x14;

const SYN_REPORT: u16 = 0x00;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;
const REL_WHEEL_HI_RES: u16 = 0x0b;
const REL_HWHEEL_HI_RES: u16 = 0x0c;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UinputSetup {
    id: InputId,
    name: [u8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputAbsinfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct UinputAbsSetup {
    code: u16,
    absinfo: InputAbsinfo,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct InputEvent {
    tv_sec: libc::time_t,
    tv_usec: libc::suseconds_t,
    type_: u16,
    code: u16,
    value: i32,
}

const fn ioc(dir: libc::c_ulong, ty: u8, nr: u8, size: usize) -> libc::Ioctl {
    ((dir << 30)
        | ((size as libc::c_ulong) << 16)
        | ((ty as libc::c_ulong) << 8)
        | (nr as libc::c_ulong)) as libc::Ioctl
}

const fn io(ty: u8, nr: u8) -> libc::Ioctl {
    ioc(0, ty, nr, 0)
}

const fn iow<T>(ty: u8, nr: u8) -> libc::Ioctl {
    ioc(1, ty, nr, size_of::<T>())
}

const UI_DEV_CREATE: libc::Ioctl = io(b'U', 1);
const UI_DEV_DESTROY: libc::Ioctl = io(b'U', 2);
const UI_DEV_SETUP: libc::Ioctl = iow::<UinputSetup>(b'U', 3);
const UI_ABS_SETUP: libc::Ioctl = iow::<UinputAbsSetup>(b'U', 4);
const UI_SET_EVBIT: libc::Ioctl = iow::<libc::c_int>(b'U', 100);
const UI_SET_KEYBIT: libc::Ioctl = iow::<libc::c_int>(b'U', 101);
const UI_SET_RELBIT: libc::Ioctl = iow::<libc::c_int>(b'U', 102);
const UI_SET_ABSBIT: libc::Ioctl = iow::<libc::c_int>(b'U', 103);

struct UinputDevice {
    file: File,
}

impl UinputDevice {
    fn open_raw() -> Result<File> {
        OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")
            .with_context(|| {
                "Failed to open /dev/uinput for writing. \
                 Run with `sudo`, or load `sudo modprobe uinput` and install `scripts/99-tether-uinput.rules`."
            })
    }

    fn emit_batch(&mut self, events: &[InputEvent]) -> Result<()> {
        let bytes = unsafe {
            std::slice::from_raw_parts(events.as_ptr() as *const u8, std::mem::size_of_val(events))
        };
        self.file
            .write_all(bytes)
            .context("Failed writing input_event batch to /dev/uinput")?;
        Ok(())
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

fn check_ioctl(ret: libc::c_int, label: &str) -> Result<()> {
    if ret < 0 {
        let err = std::io::Error::last_os_error();
        bail!("uinput ioctl {} failed: {}", label, err);
    }
    Ok(())
}

fn make_uinput_setup(name: &str, product: u16) -> UinputSetup {
    let mut name_buf = [0u8; UINPUT_MAX_NAME_SIZE];
    let c_name = CString::new(name).unwrap_or_else(|_| CString::new("TetherControl").unwrap());
    let bytes = c_name.as_bytes_with_nul();
    let copy_len = bytes.len().min(UINPUT_MAX_NAME_SIZE);
    name_buf[..copy_len].copy_from_slice(&bytes[..copy_len]);
    name_buf[UINPUT_MAX_NAME_SIZE - 1] = 0;

    UinputSetup {
        id: InputId {
            bustype: BUS_USB,
            vendor: 0x5443, // "TC"
            product,
            version: 1,
        },
        name: name_buf,
        ff_effects_max: 0,
    }
}

fn create_absolute_pointer(width: u32, height: u32) -> Result<UinputDevice> {
    let file = UinputDevice::open_raw()?;
    let fd = file.as_raw_fd();

    unsafe {
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_SYN as libc::c_int), "UI_SET_EVBIT(EV_SYN)")?;
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int), "UI_SET_EVBIT(EV_KEY)")?;
        for btn in [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE, BTN_SIDE, BTN_EXTRA] {
            check_ioctl(
                libc::ioctl(fd, UI_SET_KEYBIT, btn as libc::c_int),
                "UI_SET_KEYBIT(BTN)",
            )?;
        }

        // Absolute axes [0 .. width-1] and [0 .. height-1].
        // Paired with BTN_LEFT (and without BTN_TOUCH / INPUT_PROP_DIRECT), systemd udev input_id
        // classifies this device with ID_INPUT_MOUSE=1, matching QEMU USB Tablet behavior across Wayland & X11.
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_ABS as libc::c_int), "UI_SET_EVBIT(EV_ABS)")?;
        check_ioctl(libc::ioctl(fd, UI_SET_ABSBIT, ABS_X as libc::c_int), "UI_SET_ABSBIT(ABS_X)")?;
        check_ioctl(libc::ioctl(fd, UI_SET_ABSBIT, ABS_Y as libc::c_int), "UI_SET_ABSBIT(ABS_Y)")?;

        let abs_x = UinputAbsSetup {
            code: ABS_X,
            absinfo: InputAbsinfo {
                value: 0,
                minimum: 0,
                maximum: (width.saturating_sub(1)).max(1) as i32,
                fuzz: 0,
                flat: 0,
                resolution: 1,
            },
        };
        let abs_y = UinputAbsSetup {
            code: ABS_Y,
            absinfo: InputAbsinfo {
                value: 0,
                minimum: 0,
                maximum: (height.saturating_sub(1)).max(1) as i32,
                fuzz: 0,
                flat: 0,
                resolution: 1,
            },
        };
        check_ioctl(libc::ioctl(fd, UI_ABS_SETUP, &abs_x), "UI_ABS_SETUP(ABS_X)")?;
        check_ioctl(libc::ioctl(fd, UI_ABS_SETUP, &abs_y), "UI_ABS_SETUP(ABS_Y)")?;

        // Smooth high-resolution + discrete wheel scrolling
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_REL as libc::c_int), "UI_SET_EVBIT(EV_REL)")?;
        for rel in [REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES] {
            check_ioctl(
                libc::ioctl(fd, UI_SET_RELBIT, rel as libc::c_int),
                "UI_SET_RELBIT(WHEEL)",
            )?;
        }

        let setup = make_uinput_setup("TetherControl Absolute Pointer", 0x0001);
        check_ioctl(libc::ioctl(fd, UI_DEV_SETUP, &setup), "UI_DEV_SETUP(pointer)")?;
        check_ioctl(libc::ioctl(fd, UI_DEV_CREATE), "UI_DEV_CREATE(pointer)")?;
    }

    Ok(UinputDevice { file })
}

fn create_virtual_keyboard() -> Result<UinputDevice> {
    let file = UinputDevice::open_raw()?;
    let fd = file.as_raw_fd();

    unsafe {
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_SYN as libc::c_int), "UI_SET_EVBIT(EV_SYN)")?;
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int), "UI_SET_EVBIT(EV_KEY)")?;
        check_ioctl(libc::ioctl(fd, UI_SET_EVBIT, EV_REP as libc::c_int), "UI_SET_EVBIT(EV_REP)")?;

        for code in 1..=MAX_KEYBOARD_KEYCODE {
            check_ioctl(
                libc::ioctl(fd, UI_SET_KEYBIT, code as libc::c_int),
                "UI_SET_KEYBIT(KEY)",
            )?;
        }

        let setup = make_uinput_setup("TetherControl Virtual Keyboard", 0x0002);
        check_ioctl(libc::ioctl(fd, UI_DEV_SETUP, &setup), "UI_DEV_SETUP(keyboard)")?;
        check_ioctl(libc::ioctl(fd, UI_DEV_CREATE), "UI_DEV_CREATE(keyboard)")?;
    }

    Ok(UinputDevice { file })
}

const fn ev(type_: u16, code: u16, value: i32) -> InputEvent {
    InputEvent {
        tv_sec: 0,
        tv_usec: 0,
        type_,
        code,
        value,
    }
}

/// Manages the virtual Linux absolute pointer and virtual keyboard devices,
/// tracking held buttons/keys so disconnects or pointer-leaves cleanly release all state.
pub struct VirtualInputHub {
    pointer: UinputDevice,
    keyboard: UinputDevice,
    width: u32,
    height: u32,
    held_buttons: HashSet<u16>,
    held_keys: HashSet<u16>,
    wheel_v_remainder: i32,
    wheel_h_remainder: i32,
}

impl VirtualInputHub {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let pointer = create_absolute_pointer(width, height)?;
        let keyboard = create_virtual_keyboard()?;
        // Brief pause allowing systemd-udevd / libinput to register the newly created evdev nodes.
        thread::sleep(Duration::from_millis(150));
        info!(
            "Created Linux uinput devices: Absolute Pointer (0..{}, 0..{}) + Virtual Keyboard",
            width.saturating_sub(1),
            height.saturating_sub(1)
        );
        Ok(Self {
            pointer,
            keyboard,
            width,
            height,
            held_buttons: HashSet::new(),
            held_keys: HashSet::new(),
            wheel_v_remainder: 0,
            wheel_h_remainder: 0,
        })
    }

    pub fn move_abs(&mut self, x: u32, y: u32) -> Result<()> {
        let clamped_x = x.min(self.width.saturating_sub(1)) as i32;
        let clamped_y = y.min(self.height.saturating_sub(1)) as i32;
        let batch = [
            ev(EV_ABS, ABS_X, clamped_x),
            ev(EV_ABS, ABS_Y, clamped_y),
            ev(EV_SYN, SYN_REPORT, 0),
        ];
        self.pointer.emit_batch(&batch)
    }

    pub fn mouse_button(&mut self, btn: MouseBtn, pressed: bool) -> Result<()> {
        let code = match btn {
            MouseBtn::Left => BTN_LEFT,
            MouseBtn::Right => BTN_RIGHT,
            MouseBtn::Middle => BTN_MIDDLE,
            MouseBtn::Back => BTN_SIDE,
            MouseBtn::Forward => BTN_EXTRA,
        };
        if pressed {
            self.held_buttons.insert(code);
        } else {
            self.held_buttons.remove(&code);
        }
        let batch = [
            ev(EV_KEY, code, i32::from(pressed)),
            ev(EV_SYN, SYN_REPORT, 0),
        ];
        self.pointer.emit_batch(&batch)
    }

    pub fn scroll(&mut self, v_hi_res: i32, h_hi_res: i32) -> Result<()> {
        let mut events = Vec::with_capacity(5);

        if v_hi_res != 0 {
            events.push(ev(EV_REL, REL_WHEEL_HI_RES, v_hi_res));
            self.wheel_v_remainder += v_hi_res;
            let detents = self.wheel_v_remainder / 120;
            if detents != 0 {
                events.push(ev(EV_REL, REL_WHEEL, detents));
                self.wheel_v_remainder %= 120;
            }
        }

        if h_hi_res != 0 {
            events.push(ev(EV_REL, REL_HWHEEL_HI_RES, h_hi_res));
            self.wheel_h_remainder += h_hi_res;
            let detents = self.wheel_h_remainder / 120;
            if detents != 0 {
                events.push(ev(EV_REL, REL_HWHEEL, detents));
                self.wheel_h_remainder %= 120;
            }
        }

        if !events.is_empty() {
            events.push(ev(EV_SYN, SYN_REPORT, 0));
            self.pointer.emit_batch(&events)?;
        }
        Ok(())
    }

    pub fn key(&mut self, evdev_code: u16, pressed: bool) -> Result<()> {
        if evdev_code == 0 || evdev_code > MAX_KEYBOARD_KEYCODE {
            debug!("Ignoring out-of-range evdev keycode {}", evdev_code);
            return Ok(());
        }
        if pressed {
            self.held_keys.insert(evdev_code);
        } else {
            self.held_keys.remove(&evdev_code);
        }
        let batch = [
            ev(EV_KEY, evdev_code, i32::from(pressed)),
            ev(EV_SYN, SYN_REPORT, 0),
        ];
        self.keyboard.emit_batch(&batch)
    }

    /// Releases any currently held mouse buttons and keyboard keys.
    pub fn release_all(&mut self) {
        self.wheel_v_remainder = 0;
        self.wheel_h_remainder = 0;

        if !self.held_buttons.is_empty() {
            let mut events: Vec<InputEvent> = self
                .held_buttons
                .drain()
                .map(|code| ev(EV_KEY, code, 0))
                .collect();
            events.push(ev(EV_SYN, SYN_REPORT, 0));
            if let Err(e) = self.pointer.emit_batch(&events) {
                warn!("Failed releasing held mouse buttons: {:#}", e);
            }
        }

        if !self.held_keys.is_empty() {
            let mut events: Vec<InputEvent> = self
                .held_keys
                .drain()
                .map(|code| ev(EV_KEY, code, 0))
                .collect();
            events.push(ev(EV_SYN, SYN_REPORT, 0));
            if let Err(e) = self.keyboard.emit_batch(&events) {
                warn!("Failed releasing held keys: {:#}", e);
            }
        }
    }
}

impl Drop for VirtualInputHub {
    fn drop(&mut self) {
        self.release_all();
    }
}
