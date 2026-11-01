use crate::model::Session;
use crate::snapshot::{decode, split_key, tasks_dir};
use anyhow::Result;
use owo_colors::OwoColorize;
use std::io::{IsTerminal, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use winreg::enums::KEY_WRITE;
use winreg::RegKey;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct Options {
    pub yes: bool,
    pub dry_run: bool,
    pub kill: bool,
}

#[derive(Default)]
struct Stats {
    files: usize,
    dirs: usize,
    reg_keys: usize,
    reg_values: usize,
    reg_restored: usize,
    services: usize,
    tasks: usize,
    skipped: usize,
}

fn color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn confirm(s: &Session) -> Result<bool> {
    println!();
    println!(
        "  revert {}  ({} changes)",
        s.id,
        s.manifest.total_changes()
    );
    println!("  this deletes files and registry keys created during the run.");
    print!("  continue? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

/// Refuse obvious system locations. The manifest is ours, but a corrupted or
/// hand-edited session file should not be able to delete Windows itself.
fn safe_to_delete(p: &Path) -> bool {
    let s = p.to_string_lossy().to_lowercase();
    if p.parent().is_none() {
        return false;
    }
    if p.components().any(|c| c.as_os_str() == "..") {
        return false;
    }
    const BLOCK: &[&str] = &[
        "\\windows\\system32",
        "\\windows\\syswow64",
        "\\windows\\winsxs",
        "\\windows\\servicing",
        "\\system volume information",
        "\\$recycle.bin",
    ];
    !BLOCK.iter().any(|b| s.contains(b))
}

fn kill_processes(s: &Session) -> usize {
    let mut killed = 0;
    let created: Vec<&Path> = s
        .manifest
        .files
        .created
        .iter()
        .map(|p| p.as_path())
        .collect();

    for proc in &s.manifest.processes {
        let exe = match &proc.exe {
            Some(e) => e,
            None => continue,
        };
        let ours = created.iter().any(|c| exe.starts_with(c));
        if !ours {
            continue;
        }
        let out = Command::new("taskkill")
            .args(["/PID", &proc.pid.to_string(), "/F", "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if out.map(|o| o.status.success()).unwrap_or(false) {
            killed += 1;
        }
    }
    killed
}

fn revert_files(s: &Session, st: &mut Stats, dry: bool) {
    let mut dirs: Vec<PathBuf> = Vec::new();

    for p in &s.manifest.files.created {
        if !safe_to_delete(p) {
            st.skipped += 1;
            continue;
        }
        if !p.exists() {
            continue;
        }
        if p.is_dir() {
            dirs.push(p.clone());
            continue;
        }
        if dry {
            println!("    would delete {}", p.display());
            st.files += 1;
        } else if std::fs::remove_file(p).is_ok() {
            st.files += 1;
        } else {
            st.skipped += 1;
        }
    }

    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in dirs {
        if !d.exists() {
            continue;
        }
        if dry {
            println!("    would remove dir {}", d.display());
            st.dirs += 1;
        } else if std::fs::remove_dir(&d).is_ok() {
            st.dirs += 1;
        } else {
            st.skipped += 1;
        }
    }
}

fn revert_registry(s: &Session, st: &mut Stats, dry: bool) {
    for v in &s.manifest.registry.created_values {
        let (hive, rest) = match split_key(&v.key) {
            Some(x) => x,
            None => continue,
        };
        if dry {
            println!("    would delete value {}\\{}", v.key, v.name);
            st.reg_values += 1;
            continue;
        }
        if let Ok(k) = RegKey::predef(hive).open_subkey_with_flags(&rest, KEY_WRITE) {
            if k.delete_value(&v.name).is_ok() {
                st.reg_values += 1;
            }
        }
    }

    let mut keys = s.manifest.registry.created_keys.clone();
    keys.sort_by_key(|k| std::cmp::Reverse(k.matches('\\').count()));
    for key in keys {
        let (hive, rest) = match split_key(&key) {
            Some(x) => x,
            None => continue,
        };
        if dry {
            println!("    would delete key {}", key);
            st.reg_keys += 1;
            continue;
        }
        if RegKey::predef(hive).delete_subkey_all(&rest).is_ok() {
            st.reg_keys += 1;
        }
    }

    for ch in &s.manifest.registry.modified_values {
        let (hive, rest) = match split_key(&ch.key) {
            Some(x) => x,
            None => continue,
        };
        if dry {
            println!("    would restore {}\\{} = {}", ch.key, ch.name, ch.old);
            st.reg_restored += 1;
            continue;
        }
        if let Ok(k) = RegKey::predef(hive).open_subkey_with_flags(&rest, KEY_WRITE) {
            let rv = decode(ch.kind, &ch.old);
            if k.set_raw_value(&ch.name, &rv).is_ok() {
                st.reg_restored += 1;
            }
        }
    }
}

fn sc(args: &[&str]) -> bool {
    Command::new("sc")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn revert_services(s: &Session, st: &mut Stats, dry: bool) {
    for svc in &s.manifest.services.created {
        if dry {
            println!("    would stop and delete service {}", svc.name);
            st.services += 1;
            continue;
        }
        let _ = sc(&["stop", &svc.name]);
        if sc(&["delete", &svc.name]) {
            st.services += 1;
        } else {
            st.skipped += 1;
        }
    }
}

fn revert_tasks(s: &Session, st: &mut Stats, dry: bool) {
    let dir = tasks_dir();
    for t in &s.manifest.tasks.created {
        let p = dir.join(t);
        if dry {
            println!("    would delete task {}", p.display());
            st.tasks += 1;
            continue;
        }
        if std::fs::remove_file(&p).is_ok() {
            st.tasks += 1;
        } else {
            st.skipped += 1;
        }
    }
}

/// After reverting, check that the things we claimed to remove are gone.
fn residue(s: &Session) -> Vec<String> {
    let mut left = Vec::new();
    for p in &s.manifest.files.created {
        if p.exists() {
            left.push(p.display().to_string());
        }
    }
    for k in &s.manifest.registry.created_keys {
        if let Some((hive, rest)) = split_key(k) {
            if RegKey::predef(hive)
                .open_subkey_with_flags(&rest, winreg::enums::KEY_READ)
                .is_ok()
            {
                left.push(k.clone());
            }
        }
    }
    left
}

pub fn run(s: &Session, o: &Options) -> Result<()> {
    if !o.yes && !o.dry_run && !confirm(s)? {
        println!("  cancelled");
        return Ok(());
    }

    let mut st = Stats::default();

    if o.kill && !o.dry_run {
        let n = kill_processes(s);
        if n > 0 {
            println!("  killed {} process(es)", n);
        }
    }

    println!();
    revert_files(s, &mut st, o.dry_run);
    revert_registry(s, &mut st, o.dry_run);
    revert_services(s, &mut st, o.dry_run);
    revert_tasks(s, &mut st, o.dry_run);

    println!();
    let line = format!(
        "files {}   dirs {}   registry {} keys / {} values / {} restored   services {}   tasks {}",
        st.files, st.dirs, st.reg_keys, st.reg_values, st.reg_restored, st.services, st.tasks
    );
    if color() {
        println!("  {}", line.bold());
    } else {
        println!("  {}", line);
    }

    if !s.manifest.drivers.created.is_empty() {
        let names: Vec<String> = s
            .manifest
            .drivers
            .created
            .iter()
            .map(|d| d.name.clone())
            .collect();
        println!(
            "  {} driver(s) left in place: {}",
            names.len(),
            names.join(", ")
        );
        println!("  remove drivers by hand; they can destabilise the machine.");
    }

    if st.skipped > 0 {
        println!("  {} item(s) skipped", st.skipped);
    }

    if !o.dry_run {
        let left = residue(s);
        if left.is_empty() {
            if color() {
                println!("  {}", "clean".green());
            } else {
                println!("  clean");
            }
        } else {
            println!("  {} item(s) still present:", left.len());
            for l in left.iter().take(20) {
                println!("    {}", l);
            }
        }
    }

    println!();
    Ok(())
}
