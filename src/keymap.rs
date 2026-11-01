//! Linux `<linux/input-event-codes.h>` key and button codes, plus macOS `CGKeyCode` (`kVK_*`) translation.

pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_W: u16 = 17;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_SEMICOLON: u16 = 39;
pub const KEY_APOSTROPHE: u16 = 40;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_COMMA: u16 = 51;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_KPASTERISK: u16 = 55;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_F1: u16 = 59;
pub const KEY_F2: u16 = 60;
pub const KEY_F3: u16 = 61;
pub const KEY_F4: u16 = 62;
pub const KEY_F5: u16 = 63;
pub const KEY_F6: u16 = 64;
pub const KEY_F7: u16 = 65;
pub const KEY_F8: u16 = 66;
pub const KEY_F9: u16 = 67;
pub const KEY_F10: u16 = 68;
pub const KEY_NUMLOCK: u16 = 69;
pub const KEY_KP7: u16 = 71;
pub const KEY_KP8: u16 = 72;
pub const KEY_KP9: u16 = 73;
pub const KEY_KPMINUS: u16 = 74;
pub const KEY_KP4: u16 = 75;
pub const KEY_KP5: u16 = 76;
pub const KEY_KP6: u16 = 77;
pub const KEY_KPPLUS: u16 = 78;
pub const KEY_KP1: u16 = 79;
pub const KEY_KP2: u16 = 80;
pub const KEY_KP3: u16 = 81;
pub const KEY_KP0: u16 = 82;
pub const KEY_KPDOT: u16 = 83;
pub const KEY_102ND: u16 = 86;
pub const KEY_F11: u16 = 87;
pub const KEY_F12: u16 = 88;
pub const KEY_RO: u16 = 89;
pub const KEY_HENKAN: u16 = 92;
pub const KEY_MUHENKAN: u16 = 94;
pub const KEY_KPJPCOMMA: u16 = 95;
pub const KEY_KPENTER: u16 = 96;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_KPSLASH: u16 = 98;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;
pub const KEY_MUTE: u16 = 113;
pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_VOLUMEUP: u16 = 115;
pub const KEY_KPEQUAL: u16 = 117;
pub const KEY_YEN: u16 = 124;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;
pub const KEY_COMPOSE: u16 = 127;
pub const KEY_F13: u16 = 183;
pub const KEY_F14: u16 = 184;
pub const KEY_F15: u16 = 185;
pub const KEY_F16: u16 = 186;
pub const KEY_F17: u16 = 187;
pub const KEY_F18: u16 = 188;
pub const KEY_F19: u16 = 189;
pub const KEY_F20: u16 = 190;
pub const MAX_KEYBOARD_KEYCODE: u16 = 248;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
pub const BTN_SIDE: u16 = 0x113;
pub const BTN_EXTRA: u16 = 0x114;

/// Translates a macOS virtual keycode (`kVK_*` from `Carbon/HIToolbox/Events.h`) to a Linux `evdev` keycode.
///
/// When `swap_cmd_ctrl` is `false` (default):
/// - Mac `Control` -> Linux `Ctrl` (`KEY_LEFTCTRL` / `KEY_RIGHTCTRL`)
/// - Mac `Command` -> Linux `Super` (`KEY_LEFTMETA` / `KEY_RIGHTMETA`)
///
/// When `swap_cmd_ctrl` is `true`:
/// - Mac `Command` -> Linux `Ctrl`
/// - Mac `Control` -> Linux `Super`
pub fn macos_keycode_to_evdev(vk: u16, swap_cmd_ctrl: bool) -> Option<u16> {
    let code = match vk {
        0x00 => KEY_A,
        0x01 => KEY_S,
        0x02 => KEY_D,
        0x03 => KEY_F,
        0x04 => KEY_H,
        0x05 => KEY_G,
        0x06 => KEY_Z,
        0x07 => KEY_X,
        0x08 => KEY_C,
        0x09 => KEY_V,
        0x0A => KEY_102ND,
        0x0B => KEY_B,
        0x0C => KEY_Q,
        0x0D => KEY_W,
        0x0E => KEY_E,
        0x0F => KEY_R,
        0x10 => KEY_Y,
        0x11 => KEY_T,
        0x12 => KEY_1,
        0x13 => KEY_2,
        0x14 => KEY_3,
        0x15 => KEY_4,
        0x16 => KEY_6,
        0x17 => KEY_5,
        0x18 => KEY_EQUAL,
        0x19 => KEY_9,
        0x1A => KEY_7,
        0x1B => KEY_MINUS,
        0x1C => KEY_8,
        0x1D => KEY_0,
        0x1E => KEY_RIGHTBRACE,
        0x1F => KEY_O,
        0x20 => KEY_U,
        0x21 => KEY_LEFTBRACE,
        0x22 => KEY_I,
        0x23 => KEY_P,
        0x24 => KEY_ENTER,
        0x25 => KEY_L,
        0x26 => KEY_J,
        0x27 => KEY_APOSTROPHE,
        0x28 => KEY_K,
        0x29 => KEY_SEMICOLON,
        0x2A => KEY_BACKSLASH,
        0x2B => KEY_COMMA,
        0x2C => KEY_SLASH,
        0x2D => KEY_N,
        0x2E => KEY_M,
        0x2F => KEY_DOT,
        0x30 => KEY_TAB,
        0x31 => KEY_SPACE,
        0x32 => KEY_GRAVE,
        0x33 => KEY_BACKSPACE,
        0x35 => KEY_ESC,
        // Right Command
        0x36 => {
            if swap_cmd_ctrl {
                KEY_RIGHTCTRL
            } else {
                KEY_RIGHTMETA
            }
        }
        // Left Command
        0x37 => {
            if swap_cmd_ctrl {
                KEY_LEFTCTRL
            } else {
                KEY_LEFTMETA
            }
        }
        0x38 => KEY_LEFTSHIFT,
        0x39 => KEY_CAPSLOCK,
        0x3A => KEY_LEFTALT,
        // Left Control
        0x3B => {
            if swap_cmd_ctrl {
                KEY_LEFTMETA
            } else {
                KEY_LEFTCTRL
            }
        }
        0x3C => KEY_RIGHTSHIFT,
        0x3D => KEY_RIGHTALT,
        // Right Control
        0x3E => {
            if swap_cmd_ctrl {
                KEY_RIGHTMETA
            } else {
                KEY_RIGHTCTRL
            }
        }
        0x40 => KEY_F17,
        0x41 => KEY_KPDOT,
        0x43 => KEY_KPASTERISK,
        0x45 => KEY_KPPLUS,
        0x47 => KEY_NUMLOCK,
        0x48 => KEY_VOLUMEUP,
        0x49 => KEY_VOLUMEDOWN,
        0x4A => KEY_MUTE,
        0x4B => KEY_KPSLASH,
        0x4C => KEY_KPENTER,
        0x4E => KEY_KPMINUS,
        0x4F => KEY_F18,
        0x50 => KEY_F19,
        0x51 => KEY_KPEQUAL,
        0x52 => KEY_KP0,
        0x53 => KEY_KP1,
        0x54 => KEY_KP2,
        0x55 => KEY_KP3,
        0x56 => KEY_KP4,
        0x57 => KEY_KP5,
        0x58 => KEY_KP6,
        0x59 => KEY_KP7,
        0x5A => KEY_F20,
        0x5B => KEY_KP8,
        0x5C => KEY_KP9,
        0x5D => KEY_YEN,
        0x5E => KEY_RO,
        0x5F => KEY_KPJPCOMMA,
        0x60 => KEY_F5,
        0x61 => KEY_F6,
        0x62 => KEY_F7,
        0x63 => KEY_F3,
        0x64 => KEY_F8,
        0x65 => KEY_F9,
        0x66 => KEY_MUHENKAN,
        0x67 => KEY_F11,
        0x68 => KEY_HENKAN,
        0x69 => KEY_F13,
        0x6A => KEY_F16,
        0x6B => KEY_F14,
        0x6D => KEY_F10,
        0x6E => KEY_COMPOSE,
        0x6F => KEY_F12,
        0x71 => KEY_F15,
        0x72 => KEY_INSERT,
        0x73 => KEY_HOME,
        0x74 => KEY_PAGEUP,
        0x75 => KEY_DELETE,
        0x76 => KEY_F4,
        0x77 => KEY_END,
        0x78 => KEY_F2,
        0x79 => KEY_PAGEDOWN,
        0x7A => KEY_F1,
        0x7B => KEY_LEFT,
        0x7C => KEY_RIGHT,
        0x7D => KEY_DOWN,
        0x7E => KEY_UP,
        _ => return None,
    };
    Some(code)
}

/// Evaluates whether a macOS modifier key (`kCGEventFlagsChanged`) is now pressed based on IOKit/CGEventFlags.
///
/// Uses IOKit device-dependent left/right modifier bits (`NX_DEVICEL*KEYMASK`) when present,
/// falling back to generic `CGEventFlagMask*` bits.
pub fn modifier_is_pressed(vk: u16, flags: u64) -> Option<bool> {
    const NX_DEVICELCTLKEYMASK: u64 = 0x0000_0001;
    const NX_DEVICELSHIFTKEYMASK: u64 = 0x0000_0002;
    const NX_DEVICERSHIFTKEYMASK: u64 = 0x0000_0004;
    const NX_DEVICELCMDKEYMASK: u64 = 0x0000_0008;
    const NX_DEVICERCMDKEYMASK: u64 = 0x0000_0010;
    const NX_DEVICELALTKEYMASK: u64 = 0x0000_0020;
    const NX_DEVICERALTKEYMASK: u64 = 0x0000_0040;
    const NX_DEVICERCTLKEYMASK: u64 = 0x0000_2000;

    const CG_FLAG_ALPHASHIFT: u64 = 0x0001_0000;
    const CG_FLAG_SHIFT: u64 = 0x0002_0000;
    const CG_FLAG_CONTROL: u64 = 0x0004_0000;
    const CG_FLAG_ALTERNATE: u64 = 0x0008_0000;
    const CG_FLAG_COMMAND: u64 = 0x0010_0000;

    let any_device_bits = flags & 0x0000_207F != 0;

    match vk {
        0x37 => Some(if any_device_bits || (flags & CG_FLAG_COMMAND) == 0 {
            (flags & NX_DEVICELCMDKEYMASK) != 0
        } else {
            true
        }),
        0x36 => Some(if any_device_bits || (flags & CG_FLAG_COMMAND) == 0 {
            (flags & NX_DEVICERCMDKEYMASK) != 0
        } else {
            true
        }),
        0x38 => Some(if any_device_bits || (flags & CG_FLAG_SHIFT) == 0 {
            (flags & NX_DEVICELSHIFTKEYMASK) != 0
        } else {
            true
        }),
        0x3C => Some(if any_device_bits || (flags & CG_FLAG_SHIFT) == 0 {
            (flags & NX_DEVICERSHIFTKEYMASK) != 0
        } else {
            true
        }),
        0x3A => Some(if any_device_bits || (flags & CG_FLAG_ALTERNATE) == 0 {
            (flags & NX_DEVICELALTKEYMASK) != 0
        } else {
            true
        }),
        0x3D => Some(if any_device_bits || (flags & CG_FLAG_ALTERNATE) == 0 {
            (flags & NX_DEVICERALTKEYMASK) != 0
        } else {
            true
        }),
        0x3B => Some(if any_device_bits || (flags & CG_FLAG_CONTROL) == 0 {
            (flags & NX_DEVICELCTLKEYMASK) != 0
        } else {
            true
        }),
        0x3E => Some(if any_device_bits || (flags & CG_FLAG_CONTROL) == 0 {
            (flags & NX_DEVICERCTLKEYMASK) != 0
        } else {
            true
        }),
        0x39 => Some((flags & CG_FLAG_ALPHASHIFT) != 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_and_swapped_modifiers() {
        assert_eq!(macos_keycode_to_evdev(0x37, false), Some(KEY_LEFTMETA));
        assert_eq!(macos_keycode_to_evdev(0x3B, false), Some(KEY_LEFTCTRL));
        assert_eq!(macos_keycode_to_evdev(0x37, true), Some(KEY_LEFTCTRL));
        assert_eq!(macos_keycode_to_evdev(0x3B, true), Some(KEY_LEFTMETA));
    }

    #[test]
    fn common_alphanumeric_and_navigation_keys() {
        assert_eq!(macos_keycode_to_evdev(0x00, false), Some(KEY_A));
        assert_eq!(macos_keycode_to_evdev(0x08, false), Some(KEY_C));
        assert_eq!(macos_keycode_to_evdev(0x09, false), Some(KEY_V));
        assert_eq!(macos_keycode_to_evdev(0x24, false), Some(KEY_ENTER));
        assert_eq!(macos_keycode_to_evdev(0x33, false), Some(KEY_BACKSPACE));
        assert_eq!(macos_keycode_to_evdev(0x75, false), Some(KEY_DELETE));
        assert_eq!(macos_keycode_to_evdev(0x7B, false), Some(KEY_LEFT));
        assert_eq!(macos_keycode_to_evdev(0x7C, false), Some(KEY_RIGHT));
        assert_eq!(macos_keycode_to_evdev(0x7D, false), Some(KEY_DOWN));
        assert_eq!(macos_keycode_to_evdev(0x7E, false), Some(KEY_UP));
    }
}
