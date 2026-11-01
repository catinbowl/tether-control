use anyhow::{Context, Result, bail};
use std::io::{Read, Write};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"TETH";
pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_PORT: u16 = 9420;
pub const MDNS_SERVICE_TYPE: &str = "_tether-ctrl._tcp.local.";

/// Standard mouse buttons mapped across macOS CoreGraphics and Linux evdev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MouseBtn {
    Left = 0,
    Right = 1,
    Middle = 2,
    Back = 3,
    Forward = 4,
}

impl MouseBtn {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Left),
            1 => Some(Self::Right),
            2 => Some(Self::Middle),
            3 => Some(Self::Back),
            4 => Some(Self::Forward),
            _ => None,
        }
    }

    pub fn from_macos_button_number(btn: i64) -> Self {
        match btn {
            0 => Self::Left,
            1 => Self::Right,
            2 => Self::Middle,
            3 => Self::Back,
            _ => Self::Forward,
        }
    }
}

/// Initial handshake sent by the Linux client immediately upon TCP connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerHello {
    pub hostname: String,
    pub display_width: u32,
    pub display_height: u32,
    pub refresh_hz: u16,
    pub client_id: u32,
    pub auth_required: bool,
    pub nonce: [u8; 16],
}

/// Initial handshake reply sent by the macOS host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHello {
    pub host_name: String,
    pub auth_proof: [u8; 32],
}

/// Real-time input and lifecycle events sent from macOS Host -> Linux Client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMessage {
    /// Absolute pointer coordinates in client physical display pixels `[0..width-1, 0..height-1]`.
    MouseMove { x: u32, y: u32 },
    /// Mouse button press or release.
    MouseButton { button: MouseBtn, pressed: bool },
    /// High-resolution scroll event (`120` units = 1 discrete wheel detent).
    Scroll { v_hi_res: i32, h_hi_res: i32 },
    /// Keyboard event using Linux evdev keycode (`<linux/input-event-codes.h>`).
    Key { evdev_code: u16, pressed: bool },
    /// Host pointer entered the virtual external display at `(x, y)`.
    PointerEnter { x: u32, y: u32 },
    /// Host pointer left the virtual external display back to MacBook.
    PointerLeave,
    /// Release all currently held keys and mouse buttons on the client.
    ReleaseAll,
    /// Keepalive ping.
    Ping { seq: u32 },
    /// Graceful session disconnect.
    Disconnect,
}

/// Control messages sent from Linux Client -> macOS Host after handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientMessage {
    /// Physical display resolution changed on the client.
    DisplayChanged {
        width: u32,
        height: u32,
        refresh_hz: u16,
    },
    /// Keepalive pong matching `Ping { seq }`.
    Pong { seq: u32 },
}

/// Computes a deterministic 32-byte authentication proof from a 16-byte nonce and shared token.
/// Uses iterated SipHash-inspired mixing rounds (no external crypto C dependencies needed for LAN pairing).
pub fn compute_auth_proof(nonce: &[u8; 16], token: Option<&str>) -> [u8; 32] {
    let secret = token.unwrap_or("");
    let mut state = [
        0x736f6d6570736575_u64,
        0x646f72616e646f6d_u64,
        0x6c7967656e657261_u64,
        0x7465646279746573_u64,
    ];

    let mix = |s: &mut [u64; 4], byte: u8, idx: usize| {
        s[idx & 3] ^= (byte as u64).wrapping_mul(0x9e3779b97f4a7c15);
        for _ in 0..4 {
            s[0] = s[0].wrapping_add(s[1]);
            s[1] = s[1].rotate_left(13) ^ s[0];
            s[0] = s[0].rotate_left(32);
            s[2] = s[2].wrapping_add(s[3]);
            s[3] = s[3].rotate_left(16) ^ s[2];
            s[0] = s[0].wrapping_add(s[3]);
            s[3] = s[3].rotate_left(21) ^ s[0];
            s[2] = s[2].wrapping_add(s[1]);
            s[1] = s[1].rotate_left(17) ^ s[2];
            s[2] = s[2].rotate_left(32);
        }
    };

    for (i, &b) in nonce.iter().enumerate() {
        mix(&mut state, b, i);
    }
    mix(&mut state, 0xff, 0);
    for (i, &b) in secret.as_bytes().iter().enumerate() {
        mix(&mut state, b, i);
    }
    mix(&mut state, (secret.len() & 0xff) as u8, 3);

    let mut out = [0u8; 32];
    for (i, word) in state.iter().enumerate() {
        out[i * 8..(i + 1) * 8].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// Deterministic non-zero 32-bit FNV-1a hash used for persistent virtual display serial numbers.
pub fn stable_hash_u32(input: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in input.as_bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash == 0 { 1 } else { hash }
}

impl ServerHello {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.hostname.as_bytes();
        if name_bytes.len() > 255 {
            bail!("Hostname too long ({} bytes)", name_bytes.len());
        }
        let mut buf = Vec::with_capacity(40 + name_bytes.len());
        buf.extend_from_slice(&PROTOCOL_MAGIC);
        buf.push(PROTOCOL_VERSION);
        buf.push(name_bytes.len() as u8);
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(&self.display_width.to_be_bytes());
        buf.extend_from_slice(&self.display_height.to_be_bytes());
        buf.extend_from_slice(&self.refresh_hz.to_be_bytes());
        buf.extend_from_slice(&self.client_id.to_be_bytes());
        buf.push(u8::from(self.auth_required));
        buf.extend_from_slice(&self.nonce);
        writer.write_all(&buf)?;
        writer.flush()?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut header = [0u8; 6];
        reader
            .read_exact(&mut header)
            .context("Failed reading ServerHello header")?;
        if header[0..4] != PROTOCOL_MAGIC {
            bail!("Invalid protocol magic from server");
        }
        if header[4] != PROTOCOL_VERSION {
            bail!(
                "Protocol version mismatch: server={}, host={}",
                header[4],
                PROTOCOL_VERSION
            );
        }
        let name_len = header[5] as usize;
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let hostname = String::from_utf8(name_buf).context("Invalid UTF-8 in server hostname")?;

        let mut tail = [0u8; 4 + 4 + 2 + 4 + 1 + 16];
        reader.read_exact(&mut tail)?;
        let display_width = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
        let display_height = u32::from_be_bytes([tail[4], tail[5], tail[6], tail[7]]);
        let refresh_hz = u16::from_be_bytes([tail[8], tail[9]]);
        let client_id = u32::from_be_bytes([tail[10], tail[11], tail[12], tail[13]]);
        let auth_required = tail[14] != 0;
        let mut nonce = [0u8; 16];
        nonce.copy_from_slice(&tail[15..31]);

        if display_width == 0 || display_height == 0 || display_width > 16384 || display_height > 16384 {
            bail!(
                "Invalid client display dimensions: {}x{}",
                display_width,
                display_height
            );
        }

        Ok(Self {
            hostname,
            display_width,
            display_height,
            refresh_hz: refresh_hz.max(30),
            client_id,
            auth_required,
            nonce,
        })
    }
}

impl ClientHello {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let name_bytes = self.host_name.as_bytes();
        let truncated = &name_bytes[..name_bytes.len().min(255)];
        let mut buf = Vec::with_capacity(6 + truncated.len() + 32);
        buf.extend_from_slice(&PROTOCOL_MAGIC);
        buf.push(PROTOCOL_VERSION);
        buf.push(truncated.len() as u8);
        buf.extend_from_slice(truncated);
        buf.extend_from_slice(&self.auth_proof);
        writer.write_all(&buf)?;
        writer.flush()?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut header = [0u8; 6];
        reader
            .read_exact(&mut header)
            .context("Failed reading ClientHello header")?;
        if header[0..4] != PROTOCOL_MAGIC {
            bail!("Invalid protocol magic from host");
        }
        if header[4] != PROTOCOL_VERSION {
            bail!(
                "Protocol version mismatch: host={}, client={}",
                header[4],
                PROTOCOL_VERSION
            );
        }
        let name_len = header[5] as usize;
        let mut name_buf = vec![0u8; name_len];
        reader.read_exact(&mut name_buf)?;
        let host_name = String::from_utf8(name_buf).context("Invalid UTF-8 in host name")?;

        let mut auth_proof = [0u8; 32];
        reader.read_exact(&mut auth_proof)?;

        Ok(Self {
            host_name,
            auth_proof,
        })
    }
}

pub fn write_auth_ack<W: Write>(writer: &mut W, accepted: bool) -> Result<()> {
    writer.write_all(&[if accepted { 0x00 } else { 0x01 }])?;
    writer.flush()?;
    Ok(())
}

pub fn read_auth_ack<R: Read>(reader: &mut R) -> Result<bool> {
    let mut byte = [0u8; 1];
    reader
        .read_exact(&mut byte)
        .context("Failed reading authentication acknowledgement")?;
    Ok(byte[0] == 0x00)
}

impl HostMessage {
    /// Encodes a message into a stack buffer (max 9 bytes) with zero heap allocation.
    pub fn encode(&self, out: &mut [u8; 9]) -> usize {
        match *self {
            Self::MouseMove { x, y } => {
                out[0] = 0x01;
                out[1..5].copy_from_slice(&x.to_be_bytes());
                out[5..9].copy_from_slice(&y.to_be_bytes());
                9
            }
            Self::MouseButton { button, pressed } => {
                out[0] = 0x02;
                out[1] = button as u8;
                out[2] = u8::from(pressed);
                3
            }
            Self::Scroll { v_hi_res, h_hi_res } => {
                out[0] = 0x03;
                out[1..5].copy_from_slice(&v_hi_res.to_be_bytes());
                out[5..9].copy_from_slice(&h_hi_res.to_be_bytes());
                9
            }
            Self::Key {
                evdev_code,
                pressed,
            } => {
                out[0] = 0x04;
                out[1..3].copy_from_slice(&evdev_code.to_be_bytes());
                out[3] = u8::from(pressed);
                4
            }
            Self::PointerEnter { x, y } => {
                out[0] = 0x05;
                out[1..5].copy_from_slice(&x.to_be_bytes());
                out[5..9].copy_from_slice(&y.to_be_bytes());
                9
            }
            Self::PointerLeave => {
                out[0] = 0x06;
                1
            }
            Self::ReleaseAll => {
                out[0] = 0x07;
                1
            }
            Self::Ping { seq } => {
                out[0] = 0x08;
                out[1..5].copy_from_slice(&seq.to_be_bytes());
                5
            }
            Self::Disconnect => {
                out[0] = 0x09;
                1
            }
        }
    }

    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let mut buf = [0u8; 9];
        let len = self.encode(&mut buf);
        writer.write_all(&buf[..len])?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut tag = [0u8; 1];
        reader.read_exact(&mut tag)?;
        match tag[0] {
            0x01 => {
                let mut payload = [0u8; 8];
                reader.read_exact(&mut payload)?;
                let x = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let y = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                Ok(Self::MouseMove { x, y })
            }
            0x02 => {
                let mut payload = [0u8; 2];
                reader.read_exact(&mut payload)?;
                let button = MouseBtn::from_u8(payload[0])
                    .with_context(|| format!("Unknown mouse button code {}", payload[0]))?;
                Ok(Self::MouseButton {
                    button,
                    pressed: payload[1] != 0,
                })
            }
            0x03 => {
                let mut payload = [0u8; 8];
                reader.read_exact(&mut payload)?;
                let v_hi_res = i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let h_hi_res = i32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                Ok(Self::Scroll { v_hi_res, h_hi_res })
            }
            0x04 => {
                let mut payload = [0u8; 3];
                reader.read_exact(&mut payload)?;
                let evdev_code = u16::from_be_bytes([payload[0], payload[1]]);
                Ok(Self::Key {
                    evdev_code,
                    pressed: payload[2] != 0,
                })
            }
            0x05 => {
                let mut payload = [0u8; 8];
                reader.read_exact(&mut payload)?;
                let x = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let y = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                Ok(Self::PointerEnter { x, y })
            }
            0x06 => Ok(Self::PointerLeave),
            0x07 => Ok(Self::ReleaseAll),
            0x08 => {
                let mut payload = [0u8; 4];
                reader.read_exact(&mut payload)?;
                Ok(Self::Ping {
                    seq: u32::from_be_bytes(payload),
                })
            }
            0x09 => Ok(Self::Disconnect),
            other => bail!("Unknown HostMessage tag: 0x{:02x}", other),
        }
    }
}

impl ClientMessage {
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        match *self {
            Self::DisplayChanged {
                width,
                height,
                refresh_hz,
            } => {
                let mut buf = [0u8; 11];
                buf[0] = 0x81;
                buf[1..5].copy_from_slice(&width.to_be_bytes());
                buf[5..9].copy_from_slice(&height.to_be_bytes());
                buf[9..11].copy_from_slice(&refresh_hz.to_be_bytes());
                writer.write_all(&buf)?;
            }
            Self::Pong { seq } => {
                let mut buf = [0u8; 5];
                buf[0] = 0x88;
                buf[1..5].copy_from_slice(&seq.to_be_bytes());
                writer.write_all(&buf)?;
            }
        }
        writer.flush()?;
        Ok(())
    }

    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let mut tag = [0u8; 1];
        reader.read_exact(&mut tag)?;
        match tag[0] {
            0x81 => {
                let mut payload = [0u8; 10];
                reader.read_exact(&mut payload)?;
                let width = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let height = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let refresh_hz = u16::from_be_bytes([payload[8], payload[9]]);
                Ok(Self::DisplayChanged {
                    width,
                    height,
                    refresh_hz,
                })
            }
            0x88 => {
                let mut payload = [0u8; 4];
                reader.read_exact(&mut payload)?;
                Ok(Self::Pong {
                    seq: u32::from_be_bytes(payload),
                })
            }
            other => bail!("Unknown ClientMessage tag: 0x{:02x}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_handshake_and_auth() {
        let nonce = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let sh = ServerHello {
            hostname: "linux-workstation".into(),
            display_width: 2560,
            display_height: 1440,
            refresh_hz: 144,
            client_id: stable_hash_u32("linux-workstation"),
            auth_required: true,
            nonce,
        };
        let mut buf = Vec::new();
        sh.write_to(&mut buf).unwrap();
        let decoded_sh = ServerHello::read_from(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(sh, decoded_sh);

        let proof = compute_auth_proof(&nonce, Some("my-secret"));
        assert_ne!(proof, compute_auth_proof(&nonce, Some("wrong-secret")));

        let ch = ClientHello {
            host_name: "MacBook-Pro".into(),
            auth_proof: proof,
        };
        buf.clear();
        ch.write_to(&mut buf).unwrap();
        let decoded_ch = ClientHello::read_from(&mut Cursor::new(&buf)).unwrap();
        assert_eq!(ch, decoded_ch);
    }

    #[test]
    fn roundtrip_all_host_and_client_messages() {
        let msgs = [
            HostMessage::MouseMove { x: 1919, y: 1079 },
            HostMessage::MouseButton {
                button: MouseBtn::Left,
                pressed: true,
            },
            HostMessage::MouseButton {
                button: MouseBtn::Right,
                pressed: false,
            },
            HostMessage::Scroll {
                v_hi_res: -120,
                h_hi_res: 60,
            },
            HostMessage::Key {
                evdev_code: 30,
                pressed: true,
            },
            HostMessage::PointerEnter { x: 0, y: 540 },
            HostMessage::PointerLeave,
            HostMessage::ReleaseAll,
            HostMessage::Ping { seq: 42 },
            HostMessage::Disconnect,
        ];
        let mut wire = Vec::new();
        for m in &msgs {
            m.write_to(&mut wire).unwrap();
        }
        let mut cursor = Cursor::new(&wire);
        for expected in &msgs {
            let actual = HostMessage::read_from(&mut cursor).unwrap();
            assert_eq!(*expected, actual);
        }

        let cmsgs = [
            ClientMessage::DisplayChanged {
                width: 3840,
                height: 2160,
                refresh_hz: 120,
            },
            ClientMessage::Pong { seq: 42 },
        ];
        wire.clear();
        for m in &cmsgs {
            m.write_to(&mut wire).unwrap();
        }
        let mut cursor = Cursor::new(&wire);
        for expected in &cmsgs {
            let actual = ClientMessage::read_from(&mut cursor).unwrap();
            assert_eq!(*expected, actual);
        }
    }
}

