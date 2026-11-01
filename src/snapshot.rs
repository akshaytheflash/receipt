use crate::model::{FileDiff, NetEvent, ProcEvent, RegKind, RegSnapshot, TaskSnapshot};
use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use winreg::enums::{
    RegType, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_DWORD, REG_EXPAND_SZ,
    REG_MULTI_SZ, REG_QWORD, REG_SZ,
};
use winreg::{RegKey, RegValue};

const MAX_REG_KEYS: usize = 300_000;
const MAX_FS_EVENTS: usize = 200_000;

pub fn is_elevated() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags("SOFTWARE", KEY_WRITE)
        .is_ok()
}

// ---------------------------------------------------------------- filesystem

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Raw {
    Create,
    Modify,
    Delete,
    RenameBoth,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State {
    Created,
    Modified,
    Deleted,
}

/// Turn a flat event stream into created/modified/deleted sets.
pub fn classify(events: &[(Raw, Vec<PathBuf>)]) -> FileDiff {
    let mut m: BTreeMap<PathBuf, State> = BTreeMap::new();

    for (kind, paths) in events {
        match kind {
            Raw::Create => {
                for p in paths {
                    m.insert(p.clone(), State::Created);
                }
            }
            Raw::Modify => {
                for p in paths {
                    m.entry(p.clone()).or_insert(State::Modified);
                }
            }
            Raw::Delete => {
                for p in paths {
                    let was_created = m.get(p) == Some(&State::Created);
                    if was_created {
                        m.remove(p);
                    } else {
                        m.insert(p.clone(), State::Deleted);
                    }
                }
            }
            Raw::RenameBoth => {
                if paths.len() >= 2 {
                    let from = paths[0].clone();
                    let to = paths[1].clone();
                    if m.get(&from) == Some(&State::Created) {
                        m.remove(&from);
                    } else {
                        m.insert(from, State::Deleted);
                    }
                    m.insert(to, State::Created);
                }
            }
        }
    }

    let mut d = FileDiff::default();
    for (p, s) in m {
        match s {
            State::Created => d.created.push(p),
            State::Modified => d.modified.push(p),
            State::Deleted => d.deleted.push(p),
        }
    }
    d
}

fn noisy(p: &Path) -> bool {
    let s = p.to_string_lossy().to_lowercase();
    const NOISE: &[&str] = &[
        "\\receipt\\sessions\\",
        "\\microsoft\\edge\\",
        "\\google\\chrome\\",
        "\\google\\googleupdater\\",
        "\\mozilla\\firefox\\",
        "\\microsoft\\windows\\explorer\\",
        "\\microsoft\\search\\",
        "\\windows defender\\",
        "\\packages\\",
        "\\d3dscache",
        "\\crashdumps",
        "\\$recycle.bin",
        "\\system volume information",
        "\\node_modules\\",
        "\\microsoft\\windows\\inetcache",
        "\\microsoft\\windows\\webcache",
        "\\microsoft\\windows\\temporary internet files",
    ];
    NOISE.iter().any(|n| s.contains(n))
}

pub fn watch_roots(extra: &[PathBuf]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for k in ["ProgramFiles", "ProgramFiles(x86)", "ProgramData"] {
        if let Some(p) = std::env::var_os(k) {
            v.push(PathBuf::from(p));
        }
    }
    if let Some(p) = dirs::data_local_dir() {
        v.push(p.join("Programs"));
        v.push(p.join("Temp"));
    }
    if let Some(p) = dirs::config_dir() {
        v.push(p);
    }
    if let Some(w) = std::env::var_os("SystemRoot") {
        v.push(PathBuf::from(w).join("Temp"));
    }
    v.extend(extra.iter().cloned());
    v.retain(|p| p.exists());
    v.sort();
    v.dedup();
    v
}

pub struct FsWatch {
    watcher: Option<RecommendedWatcher>,
    rx: Receiver<notify::Result<Event>>,
}

impl FsWatch {
    pub fn start(roots: &[PathBuf]) -> Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        )?;
        for r in roots {
            // A root we cannot watch should not abort the run.
            let _ = watcher.watch(r, RecursiveMode::Recursive);
        }
        Ok(Self {
            watcher: Some(watcher),
            rx,
        })
    }

    pub fn stop(mut self) -> (FileDiff, Vec<String>) {
        self.watcher.take();
        let mut warnings = Vec::new();
        let mut events: Vec<(Raw, Vec<PathBuf>)> = Vec::new();
        let mut n = 0usize;

        while let Ok(res) = self.rx.recv() {
            let ev = match res {
                Ok(e) => e,
                Err(_) => continue,
            };
            n += 1;
            if n > MAX_FS_EVENTS {
                warnings.push(format!(
                    "file watch hit the {} event cap; later changes may be missing",
                    MAX_FS_EVENTS
                ));
                break;
            }

            let paths: Vec<PathBuf> = ev.paths.into_iter().filter(|p| !noisy(p)).collect();
            if paths.is_empty() {
                continue;
            }

            let raw = match ev.kind {
                EventKind::Create(_) => Some(Raw::Create),
                EventKind::Modify(notify::event::ModifyKind::Name(
                    notify::event::RenameMode::Both,
                )) => Some(Raw::RenameBoth),
                EventKind::Modify(_) => Some(Raw::Modify),
                EventKind::Remove(_) => Some(Raw::Delete),
                _ => None,
            };
            if let Some(r) = raw {
                events.push((r, paths));
            }
        }

        (classify(&events), warnings)
    }
}

// ------------------------------------------------------------------ registry

struct Root {
    hive: isize,
    path: &'static str,
    depth: u32,
}

fn roots() -> Vec<Root> {
    use HKEY_CURRENT_USER as CU;
    use HKEY_LOCAL_MACHINE as LM;
    vec![
        Root {
            hive: LM,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
            depth: 0,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce",
            depth: 0,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            depth: 2,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
            depth: 1,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run",
            depth: 0,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
            depth: 2,
        },
        Root {
            hive: LM,
            path: r"SOFTWARE\Classes\CLSID",
            depth: 2,
        },
        Root {
            hive: LM,
            path: r"SYSTEM\CurrentControlSet\Services",
            depth: 1,
        },
        Root {
            hive: LM,
            path: r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy\FirewallRules",
            depth: 0,
        },
        Root {
            hive: CU,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
            depth: 0,
        },
        Root {
            hive: CU,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce",
            depth: 0,
        },
        Root {
            hive: CU,
            path: r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
            depth: 2,
        },
        Root {
            hive: CU,
            path: r"SOFTWARE\Classes\CLSID",
            depth: 2,
        },
        Root {
            hive: CU,
            path: r"SOFTWARE",
            depth: 3,
        },
    ]
}

fn kind_from_type(t: RegType) -> RegKind {
    if t == REG_SZ {
        RegKind::Sz
    } else if t == REG_EXPAND_SZ {
        RegKind::ExpandSz
    } else if t == REG_DWORD {
        RegKind::Dword
    } else if t == REG_QWORD {
        RegKind::Qword
    } else if t == REG_MULTI_SZ {
        RegKind::MultiSz
    } else {
        RegKind::Binary
    }
}

pub fn type_for(kind: RegKind) -> RegType {
    match kind {
        RegKind::Sz => REG_SZ,
        RegKind::ExpandSz => REG_EXPAND_SZ,
        RegKind::Dword => REG_DWORD,
        RegKind::Qword => REG_QWORD,
        RegKind::MultiSz => REG_MULTI_SZ,
        RegKind::Binary => winreg::enums::REG_BINARY,
    }
}

fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn encode(rv: &RegValue) -> String {
    match kind_from_type(rv.vtype.clone()) {
        RegKind::Sz | RegKind::ExpandSz => {
            let mut s: Vec<u16> = utf16(&rv.bytes);
            if let Some(pos) = s.iter().position(|&c| c == 0) {
                s.truncate(pos);
            }
            String::from_utf16_lossy(&s)
        }
        RegKind::Dword => {
            if rv.bytes.len() >= 4 {
                u32::from_le_bytes([rv.bytes[0], rv.bytes[1], rv.bytes[2], rv.bytes[3]]).to_string()
            } else {
                String::new()
            }
        }
        RegKind::Qword => {
            if rv.bytes.len() >= 8 {
                let mut b = [0u8; 8];
                b.copy_from_slice(&rv.bytes[..8]);
                u64::from_le_bytes(b).to_string()
            } else {
                String::new()
            }
        }
        RegKind::MultiSz => {
            let s = utf16(&rv.bytes);
            let mut parts = Vec::new();
            let mut cur = Vec::new();
            for c in s {
                if c == 0 {
                    if cur.is_empty() {
                        break;
                    }
                    parts.push(String::from_utf16_lossy(&cur));
                    cur.clear();
                } else {
                    cur.push(c);
                }
            }
            if !cur.is_empty() {
                parts.push(String::from_utf16_lossy(&cur));
            }
            parts.join("\n")
        }
        RegKind::Binary => rv.bytes.iter().map(|b| format!("{:02x}", b)).collect(),
    }
}

pub fn decode(kind: RegKind, data: &str) -> RegValue {
    let bytes = match kind {
        RegKind::Sz | RegKind::ExpandSz => {
            let mut b: Vec<u8> = data.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            b.extend_from_slice(&[0, 0]);
            b
        }
        RegKind::Dword => data.parse::<u32>().unwrap_or(0).to_le_bytes().to_vec(),
        RegKind::Qword => data.parse::<u64>().unwrap_or(0).to_le_bytes().to_vec(),
        RegKind::MultiSz => {
            let mut b = Vec::new();
            for part in data.split('\n') {
                b.extend(part.encode_utf16().flat_map(|c| c.to_le_bytes()));
                b.extend_from_slice(&[0, 0]);
            }
            b.extend_from_slice(&[0, 0]);
            b
        }
        RegKind::Binary => (0..data.len() / 2)
            .map(|i| u8::from_str_radix(&data[i * 2..i * 2 + 2], 16).unwrap_or(0))
            .collect(),
    };
    RegValue {
        bytes,
        vtype: type_for(kind),
    }
}

pub fn snapshot_registry(warnings: &mut Vec<String>) -> RegSnapshot {
    let mut snap: RegSnapshot = BTreeMap::new();
    for r in roots() {
        let base = RegKey::predef(r.hive);
        match base.open_subkey_with_flags(r.path, KEY_READ) {
            Ok(k) => walk(
                &k,
                format!("{}\\{}", hive_name(r.hive), r.path),
                r.depth,
                &mut snap,
            ),
            Err(_) => { /* root absent on this machine; not an error */ }
        }
        if snap.len() >= MAX_REG_KEYS {
            warnings.push(format!("registry snapshot capped at {} keys", MAX_REG_KEYS));
            break;
        }
    }
    snap
}

pub fn hive_name(h: isize) -> &'static str {
    if h == HKEY_LOCAL_MACHINE {
        "HKLM"
    } else if h == HKEY_CURRENT_USER {
        "HKCU"
    } else {
        "HK??"
    }
}

fn walk(key: &RegKey, path: String, depth: u32, snap: &mut RegSnapshot) {
    if snap.len() >= MAX_REG_KEYS {
        return;
    }
    let mut vals: BTreeMap<String, (RegKind, String)> = BTreeMap::new();
    if let Ok(iter) = key.enum_values().collect::<std::io::Result<Vec<_>>>() {
        for (name, rv) in iter {
            vals.insert(name, (kind_from_type(rv.vtype.clone()), encode(&rv)));
        }
    }
    snap.insert(path.clone(), vals);

    if depth == 0 {
        return;
    }
    if let Ok(keys) = key.enum_keys().collect::<std::io::Result<Vec<_>>>() {
        for sub in keys {
            if let Ok(sk) = key.open_subkey_with_flags(&sub, KEY_READ) {
                walk(&sk, format!("{}\\{}", path, sub), depth - 1, snap);
            }
        }
    }
}

/// Split "HKLM\SOFTWARE\X" into (hive, "SOFTWARE\X").
pub fn split_key(path: &str) -> Option<(isize, String)> {
    let (hive, rest) = path.split_once('\\')?;
    let h = match hive {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        _ => return None,
    };
    Some((h, rest.to_string()))
}

// --------------------------------------------------------------------- tasks

pub fn tasks_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32")
        .join("Tasks")
}

pub fn snapshot_tasks() -> TaskSnapshot {
    let root = tasks_dir();
    let mut out = BTreeMap::new();
    walk_tasks(&root, &root, &mut out);
    out
}

fn walk_tasks(root: &Path, dir: &Path, out: &mut TaskSnapshot) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_tasks(root, &p, out);
        } else if let Ok(md) = e.metadata() {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            let mtime = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.insert(rel, (md.len(), mtime));
        }
    }
}

// ------------------------------------------------------------------- network

fn fmt_addr(ip: std::net::IpAddr, port: u16) -> String {
    match ip {
        std::net::IpAddr::V6(_) => format!("[{}]:{}", ip, port),
        _ => format!("{}:{}", ip, port),
    }
}

pub struct NetPoll {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Vec<NetEvent>>>,
}

impl NetPoll {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            use netstat2::{
                get_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo,
            };
            let mut seen: BTreeSet<(String, String, u32)> = BTreeSet::new();
            let mut out: Vec<NetEvent> = Vec::new();
            let mut baseline = true;

            while !flag.load(Ordering::Relaxed) {
                let af = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
                let pf = ProtocolFlags::TCP | ProtocolFlags::UDP;
                let mut batch: Vec<(String, String, String, u32)> = Vec::new();
                if let Ok(socks) = get_sockets_info(af, pf) {
                    for si in socks {
                        let (proto, local, remote) = match si.protocol_socket_info {
                            ProtocolSocketInfo::Tcp(t) => {
                                if t.remote_port == 0 {
                                    continue;
                                }
                                (
                                    "tcp",
                                    fmt_addr(t.local_addr, t.local_port),
                                    fmt_addr(t.remote_addr, t.remote_port),
                                )
                            }
                            ProtocolSocketInfo::Udp(u) => {
                                ("udp", fmt_addr(u.local_addr, u.local_port), "*".to_string())
                            }
                        };
                        for pid in si.associated_pids {
                            batch.push((proto.to_string(), local.clone(), remote.clone(), pid));
                        }
                    }
                }

                for (proto, local, remote, pid) in batch {
                    let fresh = seen.insert((proto.clone(), remote.clone(), pid));
                    if fresh && !baseline {
                        out.push(NetEvent {
                            proto,
                            local,
                            remote,
                            pid,
                            process: String::new(),
                        });
                    }
                }
                baseline = false;
                thread::sleep(Duration::from_millis(400));
            }
            out
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    pub fn stop(mut self) -> Vec<NetEvent> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
    }
}

// ----------------------------------------------------------------- processes

pub struct ProcPoll {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Vec<ProcEvent>>>,
}

impl ProcPoll {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::spawn(move || {
            use sysinfo::{ProcessesToUpdate, System};
            let mut sys = System::new();
            let mut seen: BTreeSet<u32> = BTreeSet::new();
            let mut out: Vec<ProcEvent> = Vec::new();
            let mut baseline = true;

            while !flag.load(Ordering::Relaxed) {
                sys.refresh_processes(ProcessesToUpdate::All, true);
                for (pid, p) in sys.processes() {
                    let id = pid.as_u32();
                    let fresh = seen.insert(id);
                    if fresh && !baseline {
                        out.push(ProcEvent {
                            pid: id,
                            ppid: p.parent().map(|x| x.as_u32()),
                            name: p.name().to_string_lossy().into_owned(),
                            exe: p.exe().map(|x| x.to_path_buf()),
                        });
                    }
                }
                baseline = false;
                thread::sleep(Duration::from_millis(400));
            }
            out
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    pub fn stop(mut self) -> Vec<ProcEvent> {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn created_then_modified_is_created() {
        let d = classify(&[(Raw::Create, vec![p("a")]), (Raw::Modify, vec![p("a")])]);
        assert_eq!(d.created, vec![p("a")]);
        assert!(d.modified.is_empty());
    }

    #[test]
    fn created_then_removed_disappears() {
        let d = classify(&[(Raw::Create, vec![p("a")]), (Raw::Delete, vec![p("a")])]);
        assert!(d.created.is_empty());
        assert!(d.deleted.is_empty());
    }

    #[test]
    fn preexisting_modified_is_modified() {
        let d = classify(&[(Raw::Modify, vec![p("a")])]);
        assert_eq!(d.modified, vec![p("a")]);
    }

    #[test]
    fn rename_counts_as_create_and_delete() {
        let d = classify(&[(Raw::RenameBoth, vec![p("old"), p("new")])]);
        assert_eq!(d.created, vec![p("new")]);
        assert_eq!(d.deleted, vec![p("old")]);
    }

    #[test]
    fn registry_roundtrip_sz() {
        let rv = decode(RegKind::Sz, "hello");
        assert_eq!(encode(&rv), "hello");
    }

    #[test]
    fn registry_roundtrip_dword() {
        let rv = decode(RegKind::Dword, "42");
        assert_eq!(encode(&rv), "42");
    }

    #[test]
    fn registry_roundtrip_multisz() {
        let rv = decode(RegKind::MultiSz, "a\nb");
        assert_eq!(encode(&rv), "a\nb");
    }

    #[test]
    fn registry_roundtrip_binary() {
        let rv = decode(RegKind::Binary, "00ff10");
        assert_eq!(encode(&rv), "00ff10");
    }
}
