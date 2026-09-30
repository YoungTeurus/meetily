//! WASAPI session PID correlation on every active endpoint and limited UI Automation.
//! Enumeration is refreshed to survive device changes and Electron child replacement.
use crate::{call_control_matches, CallState, Observation, Observer};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use windows::{
    core::{Interface, BOOL},
    Win32::{
        Foundation::{CloseHandle, FILETIME, HWND, LPARAM},
        Media::Audio::*,
        System::{Com::*, Diagnostics::ToolHelp::*, Threading::*},
        UI::{Accessibility::*, WindowsAndMessaging::*},
    },
};

struct Handle(windows::Win32::Foundation::HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
#[derive(Clone)]
struct Process {
    pid: u32,
    parent: u32,
    name: String,
    creation: u64,
}
unsafe fn process_table() -> windows::core::Result<HashMap<u32, Process>> {
    let snapshot = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    Process32FirstW(snapshot.0, &mut entry)?;
    let mut result = HashMap::new();
    loop {
        let end = entry
            .szExeFile
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(entry.szExeFile.len());
        let creation = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            entry.th32ProcessID,
        )
        .ok()
        .and_then(|h| {
            let h = Handle(h);
            let mut c = FILETIME::default();
            let mut e = FILETIME::default();
            let mut k = FILETIME::default();
            let mut u = FILETIME::default();
            GetProcessTimes(h.0, &mut c, &mut e, &mut k, &mut u)
                .ok()
                .map(|_| ((c.dwHighDateTime as u64) << 32) | (c.dwLowDateTime as u64))
        })
        .unwrap_or(0);
        result.insert(
            entry.th32ProcessID,
            Process {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase(),
                creation,
            },
        );
        if Process32NextW(snapshot.0, &mut entry).is_err() {
            break;
        }
    }
    Ok(result)
}
fn application(p: &Process) -> Option<&'static str> {
    match p.name.as_str() {
        "zoom.exe" => Some("zoom"),
        "discord.exe" | "discordcanary.exe" | "discordptb.exe" => Some("discord"),
        _ => None,
    }
}
fn owner(mut pid: u32, table: &HashMap<u32, Process>) -> Option<u32> {
    let mut root = None;
    for _ in 0..16 {
        let p = table.get(&pid)?;
        if application(p).is_some() {
            root = Some(pid);
        }
        if p.parent == 0 || p.parent == pid {
            break;
        }
        let Some(parent) = table.get(&p.parent) else {
            break;
        };
        if parent.creation > p.creation && p.creation != 0 {
            break;
        }
        pid = p.parent;
    }
    root
}
unsafe fn audio(
    table: &HashMap<u32, Process>,
) -> windows::core::Result<HashMap<u32, (bool, bool)>> {
    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let mut result = HashMap::new();
    for (flow, input) in [(eCapture, true), (eRender, false)] {
        let devices = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            for s in 0..sessions.GetCount()? {
                let control = sessions.GetSession(s)?;
                if control.GetState()? != AudioSessionStateActive {
                    continue;
                }
                let control2: IAudioSessionControl2 = control.cast()?;
                let pid = control2.GetProcessId()?;
                if let Some(root) = owner(pid, table) {
                    let entry = result.entry(root).or_insert((false, false));
                    if input {
                        entry.0 = true;
                    } else {
                        entry.1 = true;
                    }
                }
            }
        }
    }
    Ok(result)
}
struct WindowSearch<'a> {
    owner: u32,
    table: &'a HashMap<u32, Process>,
    windows: Vec<HWND>,
}
unsafe extern "system" fn window_cb(hwnd: HWND, param: LPARAM) -> BOOL {
    let search = &mut *(param.0 as *mut WindowSearch<'_>);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if IsWindowVisible(hwnd).as_bool()
        && owner(pid, search.table) == Some(search.owner)
        && search.windows.len() < 32
    {
        search.windows.push(hwnd);
    }
    BOOL(1)
}
unsafe fn children(
    node: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
) -> Vec<IUIAutomationElement> {
    let mut result = vec![];
    if let Ok(mut child) = walker.GetFirstChildElement(node) {
        for _ in 0..128 {
            result.push(child.clone());
            match walker.GetNextSiblingElement(&child) {
                Ok(next) => child = next,
                Err(_) => break,
            }
        }
    }
    result
}
unsafe fn checked_children(
    node: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
) -> Result<Vec<IUIAutomationElement>, String> {
    let no_element = windows::core::HRESULT(0x80004003u32 as i32); // null COM element => E_POINTER
    let mut result = vec![];
    let mut child = match walker.GetFirstChildElement(node) {
        Ok(c) => c,
        Err(e) if e.code() == no_element => return Ok(result),
        Err(e) => return Err(format!("Application UI children unavailable: {e}")),
    };
    loop {
        result.push(child.clone());
        match walker.GetNextSiblingElement(&child) {
            Ok(next) => {
                if result.len() >= 128 {
                    return Err(
                        "Application UI child scan limit reached; call state unknown".into(),
                    );
                }
                child = next;
            }
            Err(e) if e.code() == no_element => break,
            Err(e) => return Err(format!("Application UI siblings unavailable: {e}")),
        }
    }
    Ok(result)
}
/// Discord status text is read only next to its Disconnect control (two enclosing groups).
unsafe fn discord_status(
    button: &IUIAutomationElement,
    walker: &IUIAutomationTreeWalker,
) -> Vec<String> {
    let mut statuses = vec![];
    let started = Instant::now();
    let mut parent = button.clone();
    for _ in 0..2 {
        let Ok(p) = walker.GetParentElement(&parent) else {
            break;
        };
        parent = p;
        let mut stack = vec![(parent.clone(), 0)];
        let mut count = 0;
        while let Some((node, depth)) = stack.pop() {
            count += 1;
            if count > 48 || started.elapsed() > Duration::from_millis(300) {
                break;
            }
            let kind = node.CurrentControlType().unwrap_or(UIA_CustomControlTypeId);
            if [
                UIA_TextControlTypeId,
                UIA_StatusBarControlTypeId,
                UIA_GroupControlTypeId,
            ]
            .contains(&kind)
            {
                if let Ok(name) = node.CurrentName() {
                    statuses.push(name.to_string());
                }
            }
            if depth < 3
                && ![
                    UIA_DocumentControlTypeId,
                    UIA_EditControlTypeId,
                    UIA_ListControlTypeId,
                ]
                .contains(&kind)
            {
                for c in children(&node, walker) {
                    stack.push((c, depth + 1));
                }
            }
        }
    }
    statuses
}
unsafe fn call_ui(
    uia: &IUIAutomation,
    pid: u32,
    name: &str,
    table: &HashMap<u32, Process>,
) -> Result<bool, String> {
    let mut search = WindowSearch {
        owner: pid,
        table,
        windows: vec![],
    };
    EnumWindows(
        Some(window_cb),
        LPARAM((&mut search as *mut WindowSearch<'_>) as isize),
    )
    .map_err(|e| format!("Cannot enumerate application windows: {e}"))?;
    if search.windows.is_empty() {
        return Err(
            "Application has no accessible window; audio activity alone cannot confirm a call"
                .into(),
        );
    }
    let walker = uia
        .ControlViewWalker()
        .map_err(|e| format!("UI Automation unavailable: {e}"))?;
    let started = Instant::now();
    let mut visited = 0;
    let mut stack = vec![];
    let mut buttons = vec![];
    for window in search.windows {
        stack.push((
            uia.ElementFromHandle(window)
                .map_err(|e| format!("Application UI unavailable: {e}"))?,
            0,
        ));
    }
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if visited > 512 || started.elapsed() > Duration::from_millis(600) {
            return Err("Call-control UI scan reached its bound; call state is unknown".into());
        }
        let kind = node
            .CurrentControlType()
            .map_err(|e| format!("Cannot read UI control: {e}"))?;
        if kind == UIA_ButtonControlTypeId {
            let label = node
                .CurrentName()
                .map_err(|e| format!("Cannot read call-control name: {e}"))?
                .to_string();
            let status = if name == "discord"
                && ["disconnect", "отключиться", "отключить"]
                    .contains(&label.trim().to_lowercase().as_str())
            {
                discord_status(&node, &walker)
            } else {
                vec![]
            };
            buttons.push(label);
            if call_control_matches(name, &buttons, &status) {
                return Ok(true);
            }
        }
        // Avoid traversing chat message/document lists. The call toolbar remains in the control tree.
        if depth < 16 && ![UIA_ListControlTypeId, UIA_EditControlTypeId].contains(&kind) {
            for child in checked_children(&node, &walker)?.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    if name == "discord"
        && buttons.iter().any(|s| {
            ["disconnect", "отключиться", "отключить"].contains(&s.trim().to_lowercase().as_str())
        })
    {
        return Err("Disconnect control is present, but connected status is unavailable; call state unknown".into());
    }
    Ok(false)
}
pub struct NativeObserver {
    uia: Option<IUIAutomation>,
    com_ready: bool,
    error: Option<String>,
}
impl NativeObserver {
    pub fn new() -> Self {
        unsafe {
            let com = CoInitializeEx(None, COINIT_MULTITHREADED);
            if com.is_err() {
                return Self {
                    uia: None,
                    com_ready: false,
                    error: Some(format!("COM initialization failed: {com:?}")),
                };
            }
            match CoCreateInstance::<_, IUIAutomation>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            {
                Ok(uia) => {
                    if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
                        let _ = uia2.SetConnectionTimeout(200);
                        let _ = uia2.SetTransactionTimeout(200);
                    }
                    Self {
                        uia: Some(uia),
                        com_ready: true,
                        error: None,
                    }
                }
                Err(e) => Self {
                    uia: None,
                    com_ready: true,
                    error: Some(format!("UI Automation unavailable: {e}")),
                },
            }
        }
    }
}
impl Drop for NativeObserver {
    fn drop(&mut self) {
        self.uia.take();
        if self.com_ready {
            unsafe {
                CoUninitialize();
            }
        }
    }
}
impl Observer for NativeObserver {
    fn observe(&mut self, at: u64) -> Vec<Observation> {
        unsafe {
            let table = match process_table() {
                Ok(t) => t,
                Err(e) => {
                    return ["zoom", "discord"]
                        .iter()
                        .map(|a| {
                            Observation::unknown(a, at, &format!("Process enumeration failed: {e}"))
                        })
                        .collect()
                }
            };
            let signals = audio(&table);
            let mut result = vec![];
            for name in ["zoom", "discord"] {
                let roots: Vec<_> = table
                    .values()
                    .filter(|p| application(p) == Some(name) && owner(p.pid, &table) == Some(p.pid))
                    .collect();
                if roots.is_empty() {
                    let mut o = Observation::unknown(name, at, "");
                    o.limitations.clear();
                    o.state = CallState::NoCall;
                    o.evidence.push("Desktop application is not running".into());
                    result.push(o);
                    continue;
                }
                let mut candidates = vec![];
                for p in roots {
                    let mut o = Observation::unknown(name, at, "");
                    o.limitations.clear();
                    o.process_id = Some(p.pid);
                    if p.creation == 0 {
                        o.limitations.push("Cannot obtain process creation time; refusing unstable PID identity (check process permissions)".into());
                        candidates.push(o);
                        continue;
                    }
                    o.process_identity = Some(format!("{}-{}", p.pid, p.creation));
                    o.evidence
                        .push("Executable and process creation time identified".into());
                    let activity = signals.as_ref().ok().and_then(|s| s.get(&p.pid));
                    if let Some((input, output)) = activity {
                        if *input {
                            o.evidence.push(
                                "WASAPI capture session active for application or owned helper"
                                    .into(),
                            );
                        }
                        if *output {
                            o.evidence.push(
                                "WASAPI render session active for application or owned helper"
                                    .into(),
                            );
                        }
                    }
                    if let Err(e) = &signals {
                        o.limitations
                            .push(format!("WASAPI observation unavailable: {e}"));
                    }
                    if let Some(uia) = &self.uia {
                        match call_ui(uia, p.pid, name, &table) {
                            Ok(true) => {
                                o.state = CallState::ConfirmedCall;
                                o.confidence =
                                    if activity.is_some() { "high" } else { "medium" }.into();
                                o.evidence.push(
                                    "Application-specific call controls verified by UI Automation"
                                        .into(),
                                );
                            }
                            Ok(false) => {
                                o.state = CallState::NoCall;
                                o.evidence
                                    .push("Readable UI has no supported call controls".into());
                            }
                            Err(e) => o.limitations.push(e),
                        }
                    } else {
                        o.limitations.push(
                            self.error
                                .clone()
                                .unwrap_or_else(|| "UI Automation unavailable".into()),
                        );
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
                let mut o = Observation::unknown(
                    name,
                    at,
                    "No verified app-specific adapter; browser capture cannot identify a meeting",
                );
                o.state = CallState::Unsupported;
                result.push(o);
            }
            result
        }
    }
}
