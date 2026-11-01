use crate::model::Session;
use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::time::Duration;

fn color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn row(label: &str, value: &str) {
    if color() {
        println!("  {:<10} {}", label.dimmed(), value);
    } else {
        println!("  {:<10} {}", label, value);
    }
}

fn section(title: &str) {
    println!();
    if color() {
        println!("  {}", title.bold());
    } else {
        println!("  {}", title);
    }
}

fn counts(label: &str, parts: &[(&str, usize, &str)]) {
    let mut s = String::new();
    for (name, n, _) in parts {
        if *n > 0 {
            if !s.is_empty() {
                s.push_str("   ");
            }
            s.push_str(&format!("{} {}", n, name));
        }
    }
    if s.is_empty() {
        s.push('-');
    }
    if color() {
        println!("  {:<10} {}", label.dimmed(), s);
    } else {
        println!("  {:<10} {}", label, s);
    }
}

fn detail(title: &str, items: &[String], limit: usize) {
    if items.is_empty() {
        return;
    }
    section(title);
    for it in items.iter().take(limit) {
        println!("    {}", it);
    }
    if items.len() > limit {
        println!("    ... and {} more", items.len() - limit);
    }
}

pub fn print_session(s: &Session, elapsed: Duration) {
    println!();
    if color() {
        println!(
            "  {} {}",
            "receipt".bold(),
            env!("CARGO_PKG_VERSION").dimmed()
        );
    } else {
        println!("  receipt {}", env!("CARGO_PKG_VERSION"));
    }
    println!();
    row("session", &s.id);
    row("command", &s.command.join(" "));
    row(
        "exit",
        &s.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "killed".into()),
    );
    row("elevated", if s.elevated { "yes" } else { "no" });
    row("time", &format!("{:.1}s", elapsed.as_secs_f64()));

    let m = &s.manifest;
    section("changes");
    counts(
        "files",
        &[
            ("created", m.files.created.len(), ""),
            ("modified", m.files.modified.len(), ""),
            ("deleted", m.files.deleted.len(), ""),
        ],
    );
    counts(
        "registry",
        &[
            ("keys", m.registry.created_keys.len(), ""),
            ("values", m.registry.created_values.len(), ""),
            ("modified", m.registry.modified_values.len(), ""),
            ("removed", m.registry.deleted_values.len(), ""),
        ],
    );
    counts(
        "services",
        &[
            ("created", m.services.created.len(), ""),
            ("modified", m.services.modified.len(), ""),
        ],
    );
    counts("tasks", &[("created", m.tasks.created.len(), "")]);
    counts("drivers", &[("created", m.drivers.created.len(), "")]);
    counts("network", &[("connections", m.network.len(), "")]);
    counts("processes", &[("started", m.processes.len(), "")]);

    if m.total_changes() == 0 {
        section("no changes recorded");
        if !s.elevated {
            println!("    not elevated: HKLM and service changes were not visible");
        }
    }

    let files: Vec<String> = m
        .files
        .created
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    detail("created files", &files, 25);

    let mods: Vec<String> = m
        .files
        .modified
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    detail("modified files", &mods, 15);

    detail("created registry keys", &m.registry.created_keys, 25);

    let vals: Vec<String> = m
        .registry
        .created_values
        .iter()
        .take(40)
        .map(|v| format!("{}\\{} = {}", v.key, v.name, v.data))
        .collect();
    detail("created registry values", &vals, 40);

    let svcs: Vec<String> = m
        .services
        .created
        .iter()
        .map(|s| format!("{}  {}  [{}]", s.name, s.image, s.start))
        .collect();
    detail("created services", &svcs, 25);

    let drv: Vec<String> = m
        .drivers
        .created
        .iter()
        .map(|s| format!("{}  {}", s.name, s.image))
        .collect();
    detail("created drivers", &drv, 25);

    detail("created scheduled tasks", &m.tasks.created, 25);

    let net: Vec<String> = m
        .network
        .iter()
        .map(|n| format!("{}  {} -> {}  ({})", n.proto, n.local, n.remote, n.process))
        .collect();
    detail("network connections", &net, 25);

    if !m.warnings.is_empty() {
        section("warnings");
        for w in &m.warnings {
            if color() {
                println!("    {}", w.yellow());
            } else {
                println!("    {}", w);
            }
        }
    }

    println!();
    if color() {
        println!("  undo with {}", format!("receipt revert {}", s.id).bold());
    } else {
        println!("  undo with receipt revert {}", s.id);
    }
    println!();
}

pub fn print_list(sessions: &[Session]) {
    if sessions.is_empty() {
        println!("no sessions");
        return;
    }
    println!();
    println!("  session                exit    changes  command");
    for s in sessions {
        let exit = s
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "  {:<22} {:<8} {:>6}  {}",
            s.id,
            exit,
            s.manifest.total_changes(),
            s.command.join(" ")
        );
    }
    println!();
}
