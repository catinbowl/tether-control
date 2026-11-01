use crate::discovery::DiscoveredClient;
use crate::host::macos::event_tap::SharedTapState;
use crate::host::macos::permissions::{is_accessibility_trusted, prompt_accessibility_permission};
use crate::host::macos::virtual_display::{VirtualDisplay, VirtualDisplaySpec};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy,
    NSControlStateValueOff, NSControlStateValueOn, NSImage, NSMenu, NSMenuItem, NSRunningApplication,
    NSStatusBar, NSStatusItem, NSTextField, NSWorkspace,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, NSURL,
};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use tracing::{error, info, warn};

/// Events sent from background networking / discovery threads to the Main Thread UI & VirtualDisplay manager.
#[derive(Debug, Clone)]
pub enum UiEvent {
    Searching,
    Connecting {
        target: String,
    },
    Connected {
        hostname: String,
        addr: String,
        width: u32,
        height: u32,
        refresh_hz: u16,
        client_id: u32,
    },
    Disconnected {
        reason: Option<String>,
    },
    DiscoveredListChanged(Vec<DiscoveredClient>),
}

/// User commands sent from the Menu Bar UI to the background connection manager.
#[derive(Debug, Clone)]
pub enum NetCommand {
    ConnectManual(String),
    ConnectDiscovered(DiscoveredClient),
    EnableAutoConnect,
    Disconnect,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionPhase {
    Searching,
    Connecting(String),
    Connected {
        hostname: String,
        addr: String,
        width: u32,
        height: u32,
        refresh_hz: u16,
        display_id: Option<u32>,
    },
    Failed {
        target: String,
        reason: String,
    },
    Paused(Option<String>),
}

struct TrayState {
    mtm: MainThreadMarker,
    status_item: Retained<NSStatusItem>,
    virtual_display: Option<VirtualDisplay>,
    tap_state: Arc<SharedTapState>,
    ui_rx: Receiver<UiEvent>,
    net_tx: Sender<NetCommand>,
    phase: ConnectionPhase,
    discovered: Vec<DiscoveredClient>,
    last_manual_target: String,
    last_accessibility_ok: bool,
    last_pointer_on_client: bool,
    menu_dirty: bool,
}

struct TrayHandlerIvars {
    state: RefCell<TrayState>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TetherControlTrayHandler"]
    #[ivars = TrayHandlerIvars]
    struct TrayActionHandler;

    impl TrayActionHandler {
        #[unsafe(method(onTimerTick:))]
        fn on_timer_tick(&self, _timer: &NSTimer) {
            let Ok(mut st) = self.ivars().state.try_borrow_mut() else {
                return;
            };
            st.poll_updates(self);
        }

        #[unsafe(method(onOpenDisplaySettings:))]
        fn on_open_display_settings(&self, _sender: &NSMenuItem) {
            open_system_url("x-apple.systempreferences:com.apple.Displays-Settings.extension");
        }

        #[unsafe(method(onGrantAccessibility:))]
        fn on_grant_accessibility(&self, _sender: &NSMenuItem) {
            let trusted = prompt_accessibility_permission();
            if !trusted {
                open_system_url(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                );
            }
            if let Ok(mut st) = self.ivars().state.try_borrow_mut() {
                st.menu_dirty = true;
                st.rebuild_menu_and_icon(self);
            }
        }

        #[unsafe(method(onConnectManual:))]
        fn on_connect_manual(&self, _sender: &NSMenuItem) {
            let (mtm, default_val, net_tx) = {
                let Ok(st) = self.ivars().state.try_borrow() else {
                    return;
                };
                (st.mtm, st.last_manual_target.clone(), st.net_tx.clone())
            };

            if let Some(target) = prompt_manual_ip_dialog(mtm, &default_val) {
                if let Ok(mut st) = self.ivars().state.try_borrow_mut() {
                    st.last_manual_target = target.clone();
                    st.phase = ConnectionPhase::Connecting(target.clone());
                    st.menu_dirty = true;
                    st.rebuild_menu_and_icon(self);
                }
                let _ = net_tx.send(NetCommand::ConnectManual(target));
            }
        }

        #[unsafe(method(onSelectDiscoveredClient:))]
        fn on_select_discovered_client(&self, sender: &NSMenuItem) {
            let idx = sender.tag();
            let chosen = {
                let Ok(mut st) = self.ivars().state.try_borrow_mut() else {
                    return;
                };
                if idx < 0 || (idx as usize) >= st.discovered.len() {
                    return;
                }
                let client = st.discovered[idx as usize].clone();
                let label = client.display_label();
                st.phase = ConnectionPhase::Connecting(label);
                st.menu_dirty = true;
                let tx = st.net_tx.clone();
                st.rebuild_menu_and_icon(self);
                Some((tx, client))
            };
            if let Some((tx, client)) = chosen {
                let _ = tx.send(NetCommand::ConnectDiscovered(client));
            }
        }

        #[unsafe(method(onReconnectAuto:))]
        fn on_reconnect_auto(&self, _sender: &NSMenuItem) {
            let tx = {
                let Ok(mut st) = self.ivars().state.try_borrow_mut() else {
                    return;
                };
                st.phase = ConnectionPhase::Searching;
                st.menu_dirty = true;
                let tx = st.net_tx.clone();
                st.rebuild_menu_and_icon(self);
                tx
            };
            let _ = tx.send(NetCommand::EnableAutoConnect);
        }

        #[unsafe(method(onDisconnect:))]
        fn on_disconnect(&self, _sender: &NSMenuItem) {
            let tx = {
                let Ok(mut st) = self.ivars().state.try_borrow_mut() else {
                    return;
                };
                st.teardown_virtual_display();
                st.phase = ConnectionPhase::Paused(Some("Disconnected by user".into()));
                st.menu_dirty = true;
                let tx = st.net_tx.clone();
                st.rebuild_menu_and_icon(self);
                tx
            };
            let _ = tx.send(NetCommand::Disconnect);
        }

        #[unsafe(method(onToggleSwapCmdCtrl:))]
        fn on_toggle_swap_cmd_ctrl(&self, _sender: &NSMenuItem) {
            if let Ok(mut st) = self.ivars().state.try_borrow_mut() {
                let prev = st.tap_state.swap_cmd_ctrl.load(Ordering::Relaxed);
                st.tap_state.swap_cmd_ctrl.store(!prev, Ordering::Relaxed);
                info!("Toggled Swap Command/Control -> {}", !prev);
                st.menu_dirty = true;
                st.rebuild_menu_and_icon(self);
            }
        }

        #[unsafe(method(onOpenLogs:))]
        fn on_open_logs(&self, _sender: &NSMenuItem) {
            let log_path = crate::log_file_path();
            let path_str = log_path.to_string_lossy();
            let file_url_str = format!("file://{}", path_str);
            open_system_url(&file_url_str);
        }

        #[unsafe(method(onQuit:))]
        fn on_quit(&self, _sender: &NSMenuItem) {
            if let Ok(mut st) = self.ivars().state.try_borrow_mut() {
                st.teardown_virtual_display();
                let _ = st.net_tx.send(NetCommand::Shutdown);
                let app = NSApplication::sharedApplication(st.mtm);
                app.terminate(None);
            }
        }
    }
);

impl TrayActionHandler {
    fn new(state: TrayState) -> Retained<Self> {
        let mtm = state.mtm;
        let this = mtm.alloc::<Self>().set_ivars(TrayHandlerIvars {
            state: RefCell::new(state),
        });
        unsafe { msg_send![super(this), init] }
    }
}

impl TrayState {
    fn teardown_virtual_display(&mut self) {
        self.tap_state.clear_virtual_display();
        self.virtual_display = None;
    }

    fn poll_updates(&mut self, handler: &TrayActionHandler) {
        while let Ok(ev) = self.ui_rx.try_recv() {
            self.menu_dirty = true;
            match ev {
                UiEvent::Searching => {
                    self.teardown_virtual_display();
                    self.phase = ConnectionPhase::Searching;
                }
                UiEvent::Connecting { target } => {
                    self.phase = ConnectionPhase::Connecting(target);
                }
                UiEvent::Connected {
                    hostname,
                    addr,
                    width,
                    height,
                    refresh_hz,
                    client_id,
                } => {
                    // Register (or recreate if dimensions changed) the macOS CGVirtualDisplay
                    let need_recreate = match &self.virtual_display {
                        Some(vd) => {
                            let s = vd.spec();
                            s.width != width || s.height != height || s.serial_num != client_id
                        }
                        None => true,
                    };
                    if need_recreate {
                        self.teardown_virtual_display();
                        let spec = VirtualDisplaySpec {
                            name: format!("Tether ({})", hostname),
                            width,
                            height,
                            refresh_hz,
                            serial_num: client_id,
                        };
                        match VirtualDisplay::create(spec) {
                            Ok(vd) => {
                                let id = vd.display_id();
                                self.tap_state.set_active_virtual_display(id, width, height);
                                self.virtual_display = Some(vd);
                            }
                            Err(e) => {
                                error!("Failed to register macOS CGVirtualDisplay: {:#}", e);
                            }
                        }
                    } else if let Some(vd) = &self.virtual_display {
                        self.tap_state
                            .set_active_virtual_display(vd.display_id(), width, height);
                    }

                    let display_id = self.virtual_display.as_ref().map(|v| v.display_id());
                    self.phase = ConnectionPhase::Connected {
                        hostname,
                        addr,
                        width,
                        height,
                        refresh_hz,
                        display_id,
                    };
                }
                UiEvent::Disconnected { reason } => {
                    self.teardown_virtual_display();
                    if let Some(r) = reason {
                        let target = match &self.phase {
                            ConnectionPhase::Connecting(t) => t.clone(),
                            ConnectionPhase::Connected { addr, .. } => addr.clone(),
                            ConnectionPhase::Failed { target, .. } => target.clone(),
                            _ => String::from("client"),
                        };
                        self.phase = ConnectionPhase::Failed {
                            target,
                            reason: r,
                        };
                    } else if !matches!(self.phase, ConnectionPhase::Paused(_)) {
                        self.phase = ConnectionPhase::Searching;
                    }
                }
                UiEvent::DiscoveredListChanged(list) => {
                    self.discovered = list;
                }
            }
        }

        // Also keep bounds fresh & update icon/status if pointer crossed border or Accessibility changed
        self.tap_state.refresh_active_bounds();
        let acc_ok =
            self.tap_state.tap_installed.load(Ordering::Acquire) && is_accessibility_trusted();
        let on_client = self.tap_state.pointer_on_client.load(Ordering::Acquire);

        if acc_ok != self.last_accessibility_ok {
            self.last_accessibility_ok = acc_ok;
            self.menu_dirty = true;
        }
        if on_client != self.last_pointer_on_client {
            self.last_pointer_on_client = on_client;
            self.update_status_bar_icon();
        }

        if self.menu_dirty {
            self.menu_dirty = false;
            self.rebuild_menu_and_icon(handler);
        }
    }

    fn update_status_bar_icon(&self) {
        let Some(button) = self.status_item.button(self.mtm) else {
            return;
        };

        let symbol_name = match (&self.phase, self.last_pointer_on_client) {
            (ConnectionPhase::Connected { .. }, true) => "cursorarrow.and.square.on.square.dashed",
            (ConnectionPhase::Connected { .. }, false) => "display.2",
            _ => "display",
        };

        let sym_ns = NSString::from_str(symbol_name);
        let img = NSImage::imageWithSystemSymbolName_accessibilityDescription(&sym_ns, None)
            .or_else(|| {
                let fallback = NSString::from_str("display");
                NSImage::imageWithSystemSymbolName_accessibilityDescription(&fallback, None)
            });

        if let Some(image) = img {
            image.setTemplate(true);
            button.setImage(Some(&image));
            button.setTitle(&NSString::from_str(""));
        } else {
            let label = match (&self.phase, self.last_pointer_on_client) {
                (ConnectionPhase::Connected { .. }, true) => "TC●",
                (ConnectionPhase::Connected { .. }, false) => "TC✓",
                _ => "TC",
            };
            button.setTitle(&NSString::from_str(label));
        }
    }

    fn rebuild_menu_and_icon(&self, handler: &TrayActionHandler) {
        self.update_status_bar_icon();

        let menu = NSMenu::new(self.mtm);
        menu.setAutoenablesItems(false);

        // 1. Connection Status Header
        let status_text = match &self.phase {
            ConnectionPhase::Searching => "Status: Searching for Linux PC on LAN…".to_string(),
            ConnectionPhase::Connecting(target) => format!("Status: Connecting to {}…", target),
            ConnectionPhase::Connected {
                hostname,
                width,
                height,
                refresh_hz,
                ..
            } => format!(
                "Connected: {} ({}×{} @ {}Hz)",
                hostname, width, height, refresh_hz
            ),
            ConnectionPhase::Failed { target, reason } => {
                format!("Status: Failed to connect to {} ({})", target, reason)
            }
            ConnectionPhase::Paused(Some(reason)) => format!("Status: {}", reason),
            ConnectionPhase::Paused(None) => "Status: Disconnected".to_string(),
        };
        menu.addItem(&make_info_item(self.mtm, &status_text));

        if let ConnectionPhase::Failed { reason, .. } = &self.phase
            && (reason.contains("Connection refused") || reason.contains("os error 61"))
        {
            menu.addItem(&make_info_item(
                self.mtm,
                "  💡 Check 'tether-control client' is running on Linux",
            ));
            menu.addItem(&make_info_item(
                self.mtm,
                "  💡 Check Linux firewall allows port 9420 (sudo ufw allow 9420/tcp)",
            ));
        }

        if let ConnectionPhase::Connected {
            addr, display_id, ..
        } = &self.phase
        {
            let disp_label = match display_id {
                Some(id) => format!("Virtual Display #{} • Client {}", id, addr),
                None => format!("Virtual Display Error • Client {}", addr),
            };
            menu.addItem(&make_info_item(self.mtm, &disp_label));
        }

        // 2. Accessibility status / prompt
        if self.last_accessibility_ok {
            menu.addItem(&make_info_item(
                self.mtm,
                "✓ Input Capture Active (Ctrl+Opt+Cmd+Esc to release)",
            ));
        } else {
            let acc_item = make_action_item(
                self.mtm,
                "⚠️ Grant Accessibility Permission…",
                handler,
                sel!(onGrantAccessibility:),
                "",
            );
            menu.addItem(&acc_item);
        }

        menu.addItem(&NSMenuItem::separatorItem(self.mtm));

        // 3. Arrange Virtual Display in macOS System Settings
        let arrange_item = make_action_item(
            self.mtm,
            "Arrange Virtual Display in System Settings…",
            handler,
            sel!(onOpenDisplaySettings:),
            "",
        );
        arrange_item.setEnabled(matches!(self.phase, ConnectionPhase::Connected { .. }));
        menu.addItem(&arrange_item);

        menu.addItem(&NSMenuItem::separatorItem(self.mtm));

        // 4. Discovered LAN Clients
        menu.addItem(&make_info_item(self.mtm, "LAN Clients (mDNS)"));
        if self.discovered.is_empty() {
            menu.addItem(&make_info_item(self.mtm, "  No clients discovered yet"));
        } else {
            let active_addr = match &self.phase {
                ConnectionPhase::Connected { addr, .. } => Some(addr.as_str()),
                _ => None,
            };
            for (idx, client) in self.discovered.iter().enumerate() {
                let item = make_action_item(
                    self.mtm,
                    &client.display_label(),
                    handler,
                    sel!(onSelectDiscoveredClient:),
                    "",
                );
                item.setTag(idx as isize);
                let is_current = client
                    .addrs
                    .iter()
                    .any(|a| Some(a.to_string().as_str()) == active_addr);
                item.setState(if is_current {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                menu.addItem(&item);
            }
        }

        let manual_item = make_action_item(
            self.mtm,
            "Connect to IP / Host…",
            handler,
            sel!(onConnectManual:),
            "",
        );
        menu.addItem(&manual_item);

        match &self.phase {
            ConnectionPhase::Connected { .. } | ConnectionPhase::Connecting(_) => {
                let disc_item = make_action_item(
                    self.mtm,
                    "Disconnect",
                    handler,
                    sel!(onDisconnect:),
                    "",
                );
                menu.addItem(&disc_item);
            }
            ConnectionPhase::Paused(_) | ConnectionPhase::Failed { .. } => {
                let auto_item = make_action_item(
                    self.mtm,
                    "Resume Auto-Connect",
                    handler,
                    sel!(onReconnectAuto:),
                    "",
                );
                menu.addItem(&auto_item);
            }
            ConnectionPhase::Searching => {}
        }

        menu.addItem(&NSMenuItem::separatorItem(self.mtm));

        // 5. Modifier Key Mapping Toggle
        let swap_item = make_action_item(
            self.mtm,
            "Swap Command (⌘) and Control (⌃)",
            handler,
            sel!(onToggleSwapCmdCtrl:),
            "",
        );
        let swapped = self.tap_state.swap_cmd_ctrl.load(Ordering::Relaxed);
        swap_item.setState(if swapped {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        menu.addItem(&swap_item);

        menu.addItem(&NSMenuItem::separatorItem(self.mtm));

        // 6. View Logs
        let log_item = make_action_item(
            self.mtm,
            "View Logs…",
            handler,
            sel!(onOpenLogs:),
            "",
        );
        menu.addItem(&log_item);

        // 7. Quit
        let quit_item = make_action_item(
            self.mtm,
            "Quit TetherControl",
            handler,
            sel!(onQuit:),
            "q",
        );
        menu.addItem(&quit_item);

        self.status_item.setMenu(Some(&menu));
    }
}

fn make_info_item(mtm: MainThreadMarker, title: &str) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            None,
            &NSString::from_str(""),
        )
    };
    item.setEnabled(false);
    item
}

fn make_action_item(
    mtm: MainThreadMarker,
    title: &str,
    target: &AnyObject,
    action: objc2::runtime::Sel,
    key_equiv: &str,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key_equiv),
        )
    };
    unsafe { item.setTarget(Some(target)) };
    item.setEnabled(true);
    item
}

fn open_system_url(url_str: &str) {
    let ns_str = NSString::from_str(url_str);
    if let Some(url) = NSURL::URLWithString(&ns_str) {
        let ok = NSWorkspace::sharedWorkspace().openURL(&url);
        if !ok {
            warn!("NSWorkspace failed to open URL {}", url_str);
        }
    }
}

fn prompt_manual_ip_dialog(mtm: MainThreadMarker, initial: &str) -> Option<String> {
    // Activate app temporarily so modal alert receives keyboard focus cleanly
    let current_app = NSRunningApplication::currentApplication();
    #[allow(deprecated)]
    let _ = current_app.activateWithOptions(
        objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps,
    );

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Connect to Linux Client"));
    alert.setInformativeText(&NSString::from_str(
        "Enter the Linux PC's LAN IP address or hostname (default port 9420):",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Connect"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let input = NSTextField::initWithFrame(
        mtm.alloc(),
        CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(280.0, 24.0)),
    );
    input.setStringValue(&NSString::from_str(initial));
    input.setPlaceholderString(Some(&NSString::from_str("192.168.1.50:9420")));
    alert.setAccessoryView(Some(&input));

    let resp = alert.runModal();
    if resp == NSAlertFirstButtonReturn {
        let entered = input.stringValue().to_string();
        let trimmed = entered.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Runs the native macOS Menu Bar Tray App (`NSApplicationActivationPolicy::Accessory`) on the main thread.
pub fn run_menubar_app(
    tap_state: Arc<SharedTapState>,
    ui_rx: Receiver<UiEvent>,
    net_tx: Sender<NetCommand>,
    initial_manual_target: Option<String>,
) -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| anyhow::anyhow!("macOS Menu Bar app must run on the main thread"))?;

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // Prompt Accessibility permission once at startup if not yet granted
    let acc_ok = prompt_accessibility_permission();
    if !acc_ok {
        warn!(
            "Accessibility permission not yet granted. Grant permission in System Settings -> Privacy & Security -> Accessibility."
        );
    }

    let status_bar = NSStatusBar::systemStatusBar();
    // NSVariableStatusItemLength == -1.0
    let status_item = status_bar.statusItemWithLength(-1.0);

    let default_target = initial_manual_target.unwrap_or_else(|| "192.168.1.50:9420".into());

    let state = TrayState {
        mtm,
        status_item,
        virtual_display: None,
        tap_state,
        ui_rx,
        net_tx,
        phase: ConnectionPhase::Searching,
        discovered: Vec::new(),
        last_manual_target: default_target,
        last_accessibility_ok: acc_ok,
        last_pointer_on_client: false,
        menu_dirty: true,
    };

    let handler = TrayActionHandler::new(state);
    {
        let mut st = handler.ivars().state.borrow_mut();
        st.menu_dirty = false;
        st.rebuild_menu_and_icon(&handler);
    }

    // Schedule 80ms repeating timer on NSRunLoopCommonModes so updates run even while menu is open
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.08,
            &handler,
            sel!(onTimerTick:),
            None,
            true,
        )
    };
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }

    info!("TetherControl macOS Menu Bar app running");
    app.run();
    Ok(())
}

