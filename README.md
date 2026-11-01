# TetherControl

Control a Linux PC from a MacBook over LAN using a **native macOS Virtual External Display** mapped **1:1** to your Linux physical display.

## How It Works

1. **Linux Client (`tether-control client`)**:
   - Automatically detects your physical monitor's resolution and refresh rate via **Wayland (`wl_output`)** (with `/sys/class/drm` fallback and `--width` / `--height` CLI overrides).
   - Creates kernel-level `/dev/uinput` virtual input devices (**Absolute Pointer** `[0..W-1, 0..H-1]` + high-resolution scroll wheel + **Virtual Keyboard**), compatible out-of-the-box with Wayland (`libinput` / GNOME / KDE / Hyprland / Sway / Cosmic) and X11.
   - Advertises itself on LAN via **mDNS (`_tether-ctrl._tcp.local.`)** and listens on TCP port `9420` (`TCP_NODELAY`).

2. **macOS Host (`tether-control host`)**:
   - Runs as a native **macOS Menu Bar Tray App** (no Dock icon clutter).
   - Discovers and connects to your Linux PC automatically via mDNS (or manually via `--connect <ip[:port]>` / **"Connect to IP / Host…"** in the menu bar).
   - Upon connection, registers a **CoreGraphics Virtual External Display (`CGVirtualDisplay`)** matching the Linux monitor's exact physical resolution and refresh rate, using a deterministic serial number so **macOS System Settings -> Displays -> Arrange…** remembers your monitor layout across disconnects and reboots.
   - While your pointer is on your MacBook screen, nothing is intercepted.
   - As soon as your pointer glides onto the Virtual External Display, mouse movement (`1:1` absolute pixel mapping), mouse buttons, smooth trackpad / wheel scrolling, and keyboard events are forwarded to the Linux PC and suppressed locally on macOS.
   - When the Linux PC disconnects, the virtual display is automatically removed.

---

## 1. Setup on Linux PC (Client)

### Build

```bash
cargo build --release
```

### Grant `/dev/uinput` Access

Either run with `sudo`:

```bash
sudo ./target/release/tether-control client
```

Or configure non-root `uinput` access once using the included `udev` rule:

```bash
sudo cp scripts/99-tether-uinput.rules /etc/udev/rules.d/
echo "uinput" | sudo tee /etc/modules-load.d/uinput.conf
sudo modprobe uinput
sudo usermod -aG input "$USER"
sudo udevadm control --reload-rules && sudo udevadm trigger
# Log out and back in once so the `input` group applies, then run without sudo:
./target/release/tether-control client
```

### Linux Client CLI Options

```text
Usage: tether-control client [OPTIONS]

Options:
  -b, --bind <BIND>        Interface address to bind [default: 0.0.0.0]
  -p, --port <PORT>        TCP port to listen on [default: 9420]
      --width <WIDTH>      Override physical display width in pixels
      --height <HEIGHT>    Override physical display height in pixels
      --refresh <REFRESH>  Override physical display refresh rate in Hz
      --name <NAME>        Override advertised client hostname
      --token <TOKEN>      Optional shared authentication token
      --no-mdns            Disable mDNS LAN service advertisement
```

*(Optional)* Make sure TCP port `9420` (and UDP `5353` for mDNS) is allowed through your Linux firewall (e.g. `sudo ufw allow 9420/tcp`).

---

## 2. Setup on MacBook (Host)

### Build & Run (CLI or `.app` Bundle)

Run directly from Cargo:

```bash
cargo run --release -- host
```

Or build a standalone **`TetherControl.app`** menu bar bundle:

```bash
./scripts/bundle-macos.sh
open target/release/TetherControl.app
```

### Permissions & Display Arrangement

1. **Accessibility Permission**: On first launch, macOS will prompt for **System Settings -> Privacy & Security -> Accessibility** (needed so `CGEventTap` can capture keyboard and mouse clicks when the pointer enters the virtual display). Once toggled ON, input capture activates automatically without restarting the app.
2. **Arrange Virtual Display**: Once connected to your Linux PC, click the TetherControl display icon in the macOS Menu Bar -> **"Arrange Virtual Display in System Settings…"** (or open **System Settings -> Displays -> Arrange…**) and drag `"Tether (<linux-hostname>)"` to match where your Linux monitor physically sits relative to your MacBook (left, right, above, etc.).

### macOS Host CLI Options

```text
Usage: tether-control host [OPTIONS]

Options:
  -c, --connect <CONNECT>  Directly connect to `<ip[:port]>` instead of mDNS auto-discovery
      --swap-cmd-ctrl      Swap macOS Command (⌘) and Control (⌃) keys on Linux
      --token <TOKEN>      Optional shared authentication token matching client's `--token`
      --no-mdns            Disable mDNS LAN auto-discovery
```

### Keyboard & Safety Shortcuts

- **Default Modifier Mapping**: Mac `Control (⌃)` -> Linux `Ctrl`, Mac `Option (⌥)` -> Linux `Alt`, Mac `Command (⌘)` -> Linux `Super`.
- **Swap `⌘` and `⌃`**: Pass `--swap-cmd-ctrl` or toggle **"Swap Command (⌘) and Control (⌃)"** live from the macOS Menu Bar dropdown.
- **Emergency Release Shortcut**: Press **`Ctrl + Option + Command + Esc`** at any time while controlling Linux to release all held inputs and warp the pointer back to the center of your MacBook display.

---

## 3. Project Architecture

The codebase is organized by role (`host` and `client`) rather than tied to a single OS, making it straightforward to add Windows or cross-platform host/client support in the future:

```text
src/
├── coords.rs        # 1:1 coordinate bounding & mapping math
├── discovery.rs     # LAN mDNS advertisement & browser daemon
├── keymap.rs        # Keycode tables & macOS-to-evdev modifier mapping
├── protocol.rs      # Compact binary wire protocol & authentication
├── lib.rs           # Core library entry point
├── main.rs          # CLI dispatch (host / client)
├── host/            # Controller / Host implementation
│   ├── mod.rs       # Cross-platform host interface & CLI args
│   └── macos/       # macOS host (CGVirtualDisplay, CGEventTap, Menu Bar Tray)
└── client/          # Controlled / Client implementation
    ├── mod.rs       # Cross-platform client interface & CLI args
    └── linux/       # Linux client (Wayland/DRM display detection, uinput injection)
```

