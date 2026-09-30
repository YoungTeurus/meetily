//! Public CoreAudio process objects (macOS 14.2+) and a bounded, read-only AX adapter.
//! No taps, screen capture, keyboard hooks, or chat-message values are used.
#![allow(unexpected_cfgs)] // objc 0.2 emits obsolete cargo-clippy feature checks inside macros.
use crate::{
    call_control_matches, classify_zoom_controls, classify_zoom_window_set, zoom_meeting_command,
    CallState, Observation, Observer,
};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::{
    collections::HashMap,
    ffi::{c_char, c_void, CString},
    ptr,
};

type CFRef = *const c_void;
#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}
#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectHasProperty(object: u32, address: *const Address) -> u8;
    fn AudioObjectGetPropertyDataSize(
        object: u32,
        address: *const Address,
        qual_size: u32,
        qual: *const c_void,
        size: *mut u32,
    ) -> i32;
    fn AudioObjectGetPropertyData(
        object: u32,
        address: *const Address,
        qual_size: u32,
        qual: *const c_void,
        size: *mut u32,
        data: *mut c_void,
    ) -> i32;
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> CFRef;
    fn AXUIElementCopyAttributeValue(element: CFRef, attribute: CFRef, value: *mut CFRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: CFRef, timeout: f32) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: CFRef);
    fn CFStringCreateWithCString(allocator: CFRef, string: *const c_char, encoding: u32) -> CFRef;
    fn CFStringGetCString(string: CFRef, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
    fn CFGetTypeID(value: CFRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CFRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
    fn CFRetain(value: CFRef) -> CFRef;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: CFRef) -> bool;
}
#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "proc")]
extern "C" {
    fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut c_void, size: i32) -> i32;
}
#[repr(C)]
#[derive(Default)]
struct BsdInfo {
    flags: u32,
    status: u32,
    xstatus: u32,
    pid: u32,
    ppid: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    rfu: u32,
    comm: [u8; 16],
    name: [u8; 32],
    nfiles: u32,
    pgid: u32,
    pjobc: u32,
    tdev: u32,
    tpgid: u32,
    nice: i32,
    start_sec: u64,
    start_usec: u64,
}
struct OwnedCF(CFRef);
impl Drop for OwnedCF {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
unsafe fn cf_text(value: CFRef) -> Option<String> {
    if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
        return None;
    }
    let mut bytes = vec![0u8; 2048];
    if !CFStringGetCString(
        value,
        bytes.as_mut_ptr().cast(),
        bytes.len() as isize,
        0x08000100,
    ) {
        return None;
    }
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}
unsafe fn attr(element: CFRef, name: &str) -> Option<OwnedCF> {
    let c = CString::new(name).ok()?;
    let attribute = OwnedCF(CFStringCreateWithCString(
        ptr::null(),
        c.as_ptr(),
        0x08000100,
    ));
    let mut value = ptr::null();
    (AXUIElementCopyAttributeValue(element, attribute.0, &mut value) == 0 && !value.is_null())
        .then_some(OwnedCF(value))
}
unsafe fn text_attr(element: CFRef, name: &str) -> Option<String> {
    attr(element, name).and_then(|v| cf_text(v.0))
}
fn info(pid: u32) -> Option<BsdInfo> {
    let mut i = BsdInfo::default();
    let n = unsafe {
        proc_pidinfo(
            pid as i32,
            3,
            0,
            (&mut i as *mut BsdInfo).cast(),
            std::mem::size_of::<BsdInfo>() as i32,
        )
    };
    (n == std::mem::size_of::<BsdInfo>() as i32).then_some(i)
}
fn address(selector: u32) -> Address {
    Address {
        selector,
        scope: u32::from_be_bytes(*b"glob"),
        element: 0,
    }
}
unsafe fn scalar(object: u32, selector: u32) -> Option<u32> {
    let mut v = 0;
    let mut size = 4;
    (AudioObjectGetPropertyData(
        object,
        &address(selector),
        0,
        ptr::null(),
        &mut size,
        (&mut v as *mut u32).cast(),
    ) == 0)
        .then_some(v)
}
unsafe fn processes() -> Result<Vec<u32>, String> {
    let a = address(u32::from_be_bytes(*b"prs#"));
    if AudioObjectHasProperty(1, &a) == 0 {
        return Err(
            "CoreAudio process observation requires macOS 14.2 or later (14.8.5 is supported)"
                .into(),
        );
    }
    let mut size = 0;
    if AudioObjectGetPropertyDataSize(1, &a, 0, ptr::null(), &mut size) != 0 {
        return Err("CoreAudio process list unavailable".into());
    }
    let mut values = vec![0; size as usize / 4];
    if AudioObjectGetPropertyData(1, &a, 0, ptr::null(), &mut size, values.as_mut_ptr().cast()) != 0
    {
        return Err("CoreAudio process list changed while reading".into());
    }
    values.truncate(size as usize / 4);
    Ok(values)
}
#[derive(Clone)]
struct App {
    name: String,
    pid: u32,
    identity: Option<String>,
}
unsafe fn running_apps() -> Vec<App> {
    let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
    let apps: *mut Object = msg_send![ws, runningApplications];
    let count: usize = msg_send![apps, count];
    let mut result = vec![];
    for index in 0..count {
        let app: *mut Object = msg_send![apps,objectAtIndex:index];
        let bundle: *mut Object = msg_send![app, bundleIdentifier];
        if bundle.is_null() {
            continue;
        }
        let c: *const c_char = msg_send![bundle, UTF8String];
        if c.is_null() {
            continue;
        }
        let b = std::ffi::CStr::from_ptr(c).to_string_lossy();
        let name = match b.as_ref() {
            "us.zoom.xos" => "zoom",
            "com.hnc.Discord" | "com.hnc.DiscordCanary" | "com.hnc.DiscordPTB" => "discord",
            _ => continue,
        };
        let pid: i32 = msg_send![app, processIdentifier];
        // Never accept a PID without a stable OS creation identity.
        result.push(App {
            name: name.into(),
            pid: pid as u32,
            identity: info(pid as u32).map(|i| format!("{pid}-{}-{}", i.start_sec, i.start_usec)),
        });
    }
    result
}
fn owner(mut pid: u32, apps: &[App]) -> Option<u32> {
    for _ in 0..12 {
        if apps.iter().any(|a| a.pid == pid) {
            return Some(pid);
        };
        let i = info(pid)?;
        if i.ppid == pid || i.ppid == 0 {
            break;
        }
        if let Some(parent) = info(i.ppid) {
            if (parent.start_sec, parent.start_usec) > (i.start_sec, i.start_usec) {
                break;
            }
        }
        pid = i.ppid;
    }
    None
}
unsafe fn discord_status(button: CFRef) -> Vec<String> {
    let mut names = vec![];
    let started = std::time::Instant::now();
    let mut parent = OwnedCF(CFRetain(button));
    // Inspect static call-status text only in the two groups enclosing Disconnect.
    for _ in 0..2 {
        let Some(p) = attr(parent.0, "AXParent") else {
            break;
        };
        parent = p;
        let mut stack = vec![(OwnedCF(CFRetain(parent.0)), 0)];
        let mut count = 0;
        while let Some((node, depth)) = stack.pop() {
            count += 1;
            if count > 48 || started.elapsed() > std::time::Duration::from_millis(300) {
                break;
            }
            let role = text_attr(node.0, "AXRole").unwrap_or_default();
            if role == "AXStaticText" {
                for k in ["AXValue", "AXDescription"] {
                    if let Some(s) = text_attr(node.0, k) {
                        names.push(s);
                    }
                }
            }
            if depth < 3 && !["AXTextArea", "AXList", "AXTable"].contains(&role.as_str()) {
                if let Some(children) = attr(node.0, "AXChildren") {
                    if CFGetTypeID(children.0) == CFArrayGetTypeID() {
                        for i in 0..CFArrayGetCount(children.0).min(32) {
                            stack.push((
                                OwnedCF(CFRetain(CFArrayGetValueAtIndex(children.0, i))),
                                depth + 1,
                            ));
                        }
                    }
                }
            }
        }
    }
    names
}
unsafe fn zoom_menu_commands(root: CFRef) -> Option<Vec<(String, bool)>> {
    let menu = attr(root, "AXMenuBar")?;
    let mut stack = vec![(menu, 0)];
    let mut commands = vec![];
    let mut count = 0;
    let start = std::time::Instant::now();
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if count > 192 || start.elapsed() > std::time::Duration::from_millis(200) {
            return None;
        }
        let role = text_attr(node.0, "AXRole")?;
        if role == "AXMenuItem" {
            if let Some(title) = text_attr(node.0, "AXTitle") {
                if zoom_meeting_command(&title).is_some() {
                    let enabled = attr(node.0, "AXEnabled")?;
                    if CFGetTypeID(enabled.0) != CFBooleanGetTypeID() {
                        return None;
                    }
                    commands.push((title, CFBooleanGetValue(enabled.0)));
                    if commands
                        .last()
                        .map(|(_, enabled)| *enabled)
                        .unwrap_or(false)
                    {
                        return Some(commands);
                    }
                }
            }
        }
        let children = attr(node.0, "AXChildren");
        if children.is_none() && ["AXMenuBar", "AXMenu", "AXMenuBarItem"].contains(&role.as_str()) {
            return None;
        }
        if let Some(children) = children {
            if CFGetTypeID(children.0) != CFArrayGetTypeID() {
                return None;
            }
            let size = CFArrayGetCount(children.0);
            if size > 128 || (depth >= 5 && size > 0) {
                return None;
            }
            for i in (0..size).rev() {
                stack.push((
                    OwnedCF(CFRetain(CFArrayGetValueAtIndex(children.0, i))),
                    depth + 1,
                ));
            }
        }
    }
    Some(commands)
}
unsafe fn controls(pid: u32, application: &str) -> Result<CallState, String> {
    let root = OwnedCF(AXUIElementCreateApplication(pid as i32));
    AXUIElementSetMessagingTimeout(root.0, 0.15);
    let menu_commands = if application == "zoom" {
        zoom_menu_commands(root.0)
    } else {
        None
    };
    if application == "zoom"
        && classify_zoom_controls(&[], menu_commands.as_deref()) == CallState::ConfirmedCall
    {
        return Ok(CallState::ConfirmedCall);
    }
    // AXWindows existence distinguishes inaccessible/unresponsive from an empty readable tree.
    let windows = attr(root.0, "AXWindows")
        .ok_or("App accessibility tree unavailable; detection cannot confirm call state")?;
    if CFGetTypeID(windows.0) != CFArrayGetTypeID() {
        return Err("Invalid app accessibility tree".into());
    }
    if CFArrayGetCount(windows.0) == 0 {
        return Err("Application has no accessible window; call state is unknown".into());
    }
    let mut stack = vec![];
    for i in 0..CFArrayGetCount(windows.0).min(16) {
        stack.push((OwnedCF(CFRetain(CFArrayGetValueAtIndex(windows.0, i))), 0));
    }
    let mut buttons = vec![];
    let mut status = vec![];
    let mut visited = 0;
    let started = std::time::Instant::now();
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > 512 || started.elapsed() > std::time::Duration::from_millis(500) {
            return Err("Call-control accessibility scan reached its bound; state unknown".into());
        }
        let role = text_attr(node.0, "AXRole").ok_or(
            "Accessibility provider did not expose a readable control role; call state is unknown",
        )?;
        if role == "AXButton" || role == "AXMenuButton" {
            for key in ["AXTitle", "AXDescription", "AXHelp"] {
                if let Some(s) = text_attr(node.0, key) {
                    // Idle evidence must come from enabled controls; a modal/live
                    // meeting can leave disabled Home controls in the AX tree.
                    let label = s.trim().to_lowercase();
                    if [
                        "join",
                        "join meeting",
                        "войти",
                        "войти в конференцию",
                        "new meeting",
                        "новая конференция",
                    ]
                    .contains(&label.as_str())
                    {
                        let Some(enabled) = attr(node.0, "AXEnabled") else {
                            continue;
                        };
                        if CFGetTypeID(enabled.0) != CFBooleanGetTypeID()
                            || !CFBooleanGetValue(enabled.0)
                        {
                            continue;
                        }
                    }
                    buttons.push(s);
                }
            }
            if application == "discord"
                && buttons.iter().rev().take(3).any(|s| {
                    ["disconnect", "отключиться", "отключить"]
                        .contains(&s.trim().to_lowercase().as_str())
                })
            {
                status.extend(discord_status(node.0));
            }
        } else if role == "AXGroup" || role == "AXToolbar" || role == "AXStaticText" {
            // Global scan reads labels only; AXValue is confined to the Disconnect group above.
            if let Some(s) = text_attr(node.0, "AXDescription") {
                status.push(s);
            }
        }
        if call_control_matches(application, &buttons, &status) {
            return Ok(CallState::ConfirmedCall);
        }
        if depth < 14 && !["AXTextArea", "AXList", "AXTable"].contains(&role.as_str()) {
            let children_attribute = attr(node.0, "AXChildren");
            if children_attribute.is_none()
                && [
                    "AXWindow",
                    "AXGroup",
                    "AXWebArea",
                    "AXScrollArea",
                    "AXSplitGroup",
                ]
                .contains(&role.as_str())
            {
                return Err(
                    "Application accessibility container is unreadable; call state is unknown"
                        .into(),
                );
            }
            if let Some(children) = children_attribute {
                if CFGetTypeID(children.0) == CFArrayGetTypeID() {
                    if CFArrayGetCount(children.0) > 128 {
                        return Err("Application accessibility child scan limit reached; call state unknown".into());
                    }
                    for i in (0..CFArrayGetCount(children.0).min(128)).rev() {
                        stack.push((
                            OwnedCF(CFRetain(CFArrayGetValueAtIndex(children.0, i))),
                            depth + 1,
                        ));
                    }
                }
            }
        }
    }
    if application == "discord"
        && buttons.iter().any(|s| {
            ["disconnect", "отключиться", "отключить"].contains(&s.trim().to_lowercase().as_str())
        })
    {
        return Err("Disconnect control is present, but connected status is unavailable; call state unknown".into());
    }
    if application == "zoom" {
        Ok(classify_zoom_window_set(
            &buttons,
            menu_commands.as_deref(),
            CFArrayGetCount(windows.0) as usize,
        ))
    } else {
        Ok(CallState::NoCall)
    }
}
pub struct NativeObserver;
impl NativeObserver {
    pub fn new() -> Self {
        Self
    }
}
impl Observer for NativeObserver {
    fn observe(&mut self, at: u64) -> Vec<Observation> {
        unsafe {
            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            let apps = running_apps();
            let audio = processes();
            let trusted = AXIsProcessTrusted();
            let mut signals: HashMap<u32, (bool, bool)> = HashMap::new();
            if let Ok(objects) = &audio {
                for object in objects {
                    if let Some(pid) = scalar(*object, u32::from_be_bytes(*b"ppid")) {
                        if let Some(root) = owner(pid, &apps) {
                            let entry = signals.entry(root).or_default();
                            entry.0 |=
                                scalar(*object, u32::from_be_bytes(*b"piri")).unwrap_or(0) != 0;
                            entry.1 |=
                                scalar(*object, u32::from_be_bytes(*b"piro")).unwrap_or(0) != 0;
                        }
                    }
                }
            }
            let mut result = vec![];
            for name in ["zoom", "discord"] {
                let matching: Vec<_> = apps.iter().filter(|a| a.name == name).collect();
                if matching.is_empty() {
                    let mut o = Observation::unknown(name, at, "");
                    o.limitations.clear();
                    o.state = CallState::NoCall;
                    o.evidence.push("Desktop application is not running".into());
                    result.push(o);
                    continue;
                }
                // Multiple Discord channels/versions are evaluated independently, with confirmed taking precedence.
                let mut candidates = vec![];
                for app in matching {
                    let mut o = Observation::unknown(name, at, "");
                    o.limitations.clear();
                    o.process_id = Some(app.pid);
                    o.process_identity = app.identity.clone();
                    if o.process_identity.is_none() {
                        o.limitations.push("Registered application is running, but its process creation time is unavailable; refusing an unstable PID identity".into());
                        candidates.push(o);
                        continue;
                    }
                    o.evidence.push(
                        "Application identified by registered bundle ID and process creation time"
                            .into(),
                    );
                    if let Some((input, output)) = signals.get(&app.pid) {
                        if *input {
                            o.evidence.push("CoreAudio process input is running (includes owned helper processes)".into());
                        }
                        if *output {
                            o.evidence.push("CoreAudio process output is running (includes owned helper processes)".into());
                        }
                    }
                    if let Err(e) = &audio {
                        o.limitations.push(e.clone());
                    }
                    if !trusted {
                        o.limitations.push("Accessibility permission is required to distinguish a call from music, microphone tests, and text chat. Enable Meetily in System Settings → Privacy & Security → Accessibility.".into());
                    } else {
                        match controls(app.pid, name) {
                            Ok(CallState::ConfirmedCall) => {
                                o.state = CallState::ConfirmedCall;
                                o.confidence = if signals
                                    .get(&app.pid)
                                    .map(|(i, o)| *i || *o)
                                    .unwrap_or(false)
                                {
                                    "high"
                                } else {
                                    "medium"
                                }
                                .into();
                                o.evidence.push("Application call-specific controls or enabled meeting commands verified in Accessibility".into());
                            }
                            Ok(CallState::NoCall) => {
                                o.state = CallState::NoCall;
                                o.evidence.push(
                                    "Readable application UI confirms the idle home controls with no enabled meeting command".into(),
                                );
                            }
                            Ok(_)=>o.limitations.push("Zoom call controls are temporarily not exposed; absence of a toolbar is not an observed exit".into()),
                            Err(e) => o.limitations.push(e),
                        }
                    }
                    candidates.push(o);
                }
                candidates.sort_by_key(|o| match o.state {
                    CallState::ConfirmedCall => 0,
                    CallState::Unknown => 1,
                    _ => 2,
                });
                result.push(candidates.remove(0));
            }
            for name in ["teams", "browser"] {
                let mut o=Observation::unknown(name,at,"No verified app-specific call controls; browser microphone activity does not identify a meeting");
                o.state = CallState::Unsupported;
                result.push(o);
            }
            let _: () = msg_send![pool, drain];
            result
        }
    }
}

/// Called only after an explicit Start click. Foreground restoration is a fallback,
/// and fresh app-specific evidence plus the original creation identity are mandatory.
pub fn revalidate(application: &str, identity: &str, at: u64) -> Result<Observation, String> {
    unsafe {
        let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
        let result = (|| {
            let original = running_apps()
                .into_iter()
                .find(|a| a.name == application && a.identity.as_deref() == Some(identity))
                .ok_or("Call application exited or its process identity changed")?;
            let mut observer = NativeObserver::new();
            let observation = observer
                .observe(at)
                .into_iter()
                .find(|o| o.application == application)
                .ok_or("Application observation unavailable")?;
            if observation.process_identity.as_deref() != Some(identity) {
                return Err("Call application process identity changed".into());
            }
            if observation.state == CallState::ConfirmedCall
                || observation.state == CallState::NoCall
            {
                return Ok(observation);
            }
            let app: *mut Object = msg_send![class!(NSRunningApplication),runningApplicationWithProcessIdentifier:original.pid as i32];
            if app.is_null() {
                return Err("Call application exited".into());
            }
            let activated: bool = msg_send![app,activateWithOptions:2usize];
            if !activated {
                return Err("Could not show the call application for verification".into());
            }
            let start = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let fresh_at = at.saturating_add(start.elapsed().as_millis() as u64);
                let observation = observer
                    .observe(fresh_at)
                    .into_iter()
                    .find(|o| o.application == application)
                    .ok_or("Application observation unavailable")?;
                if observation.process_identity.as_deref() != Some(identity) {
                    return Err("Call application exited or its process identity changed".into());
                }
                if observation.state != CallState::Unknown
                    || start.elapsed() >= std::time::Duration::from_secs(3)
                {
                    return Ok(observation);
                }
            }
        })();
        let _: () = msg_send![pool, drain];
        result
    }
}
