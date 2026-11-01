use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use receipt::model::{Manifest, Session};
use receipt::{diff, report, revert, session, snapshot};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "receipt",
    version,
    about = "Run an installer, see everything it changed, undo it."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a command, record what it changes, save the manifest.
    Run(RunArgs),
    /// List recorded sessions.
    List(ListArgs),
    /// Show a session's manifest (default: the latest).
    Show(ShowArgs),
    /// Undo a session.
    Revert(RevertArgs),
}

#[derive(Args)]
#[command(trailing_var_arg = true)]
struct RunArgs {
    /// Label for this run.
    #[arg(long)]
    name: Option<String>,
    /// Print only the manifest as JSON.
    #[arg(long)]
    json: bool,
    /// Kill the child after this many seconds.
    #[arg(long, default_value_t = 1800)]
    timeout: u64,
    /// Extra directory to watch (repeatable).
    #[arg(long = "watch", value_name = "DIR")]
    watch: Vec<PathBuf>,
    /// Skip the network poller.
    #[arg(long)]
    no_network: bool,
    /// Skip the process poller.
    #[arg(long)]
    no_processes: bool,
    /// Command and arguments. Put receipt's own flags before the command.
    #[arg(required = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ShowArgs {
    /// Session id. Defaults to the latest.
    id: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct RevertArgs {
    /// Session id. Defaults to the latest.
    id: Option<String>,
    /// Do not ask for confirmation.
    #[arg(long)]
    yes: bool,
    /// Print what would happen, change nothing.
    #[arg(long)]
    dry_run: bool,
    /// Kill processes that started from created paths.
    #[arg(long)]
    kill: bool,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.cmd {
        Cmd::Run(a) => run(a),
        Cmd::List(a) => list(a),
        Cmd::Show(a) => show(a),
        Cmd::Revert(a) => revert_cmd(a),
    };
    if let Err(e) = result {
        eprintln!("receipt: {e:#}");
        std::process::exit(1);
    }
}

fn run(a: RunArgs) -> Result<()> {
    let elevated = snapshot::is_elevated();
    let id = session::new_id();
    let started = Utc::now();
    let t0 = Instant::now();

    let mut warnings = Vec::new();
    if !elevated {
        warnings.push("not elevated: HKLM writes and service changes are not visible".to_string());
    }

    let roots = snapshot::watch_roots(&a.watch);
    if roots.is_empty() {
        warnings.push("no watchable directories found".to_string());
    }

    let reg_before = snapshot::snapshot_registry(&mut warnings);
    let tasks_before = snapshot::snapshot_tasks();

    let fs = snapshot::FsWatch::start(&roots)?;
    let net = if a.no_network {
        None
    } else {
        Some(snapshot::NetPoll::start())
    };
    let procs = if a.no_processes {
        None
    } else {
        Some(snapshot::ProcPoll::start())
    };

    // Give the watcher a moment to attach before the child can write.
    std::thread::sleep(Duration::from_millis(400));

    let mut child = Command::new(&a.command[0])
        .args(&a.command[1..])
        .spawn()
        .with_context(|| format!("could not start {}", a.command[0]))?;

    let exit = wait_timeout(&mut child, a.timeout)?;

    let (files, fs_warnings) = fs.stop();
    warnings.extend(fs_warnings);

    let mut network = net.map(|n| n.stop()).unwrap_or_default();
    let processes = procs.map(|p| p.stop()).unwrap_or_default();

    let reg_after = snapshot::snapshot_registry(&mut warnings);
    let tasks_after = snapshot::snapshot_tasks();

    let names: HashMap<u32, String> = processes.iter().map(|p| (p.pid, p.name.clone())).collect();
    for n in network.iter_mut() {
        n.process = names
            .get(&n.pid)
            .cloned()
            .unwrap_or_else(|| "unknown".into());
    }
    network.sort_by(|a, b| a.remote.cmp(&b.remote));

    let manifest = Manifest {
        files,
        registry: diff::registry(&reg_before, &reg_after),
        services: diff::services(&reg_before, &reg_after),
        tasks: diff::tasks(&tasks_before, &tasks_after),
        drivers: diff::drivers(&reg_before, &reg_after),
        network,
        processes,
        warnings,
    };

    let s = Session {
        id,
        started,
        ended: Utc::now(),
        duration_ms: t0.elapsed().as_millis() as u64,
        command: a.command.clone(),
        name: a.name,
        exit_code: exit,
        elevated,
        host: std::env::var("COMPUTERNAME").unwrap_or_default(),
        manifest,
    };

    session::save(&s)?;

    if a.json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        report::print_session(&s, t0.elapsed());
    }
    Ok(())
}

fn wait_timeout(child: &mut Child, secs: u64) -> Result<Option<i32>> {
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok(st.code());
        }
        if start.elapsed() > Duration::from_secs(secs) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn list(a: ListArgs) -> Result<()> {
    let sessions = session::list()?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
    } else {
        report::print_list(&sessions);
    }
    Ok(())
}

fn show(a: ShowArgs) -> Result<()> {
    let s = match a.id {
        Some(id) => session::load(&id)?,
        None => session::latest()?,
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&s)?);
    } else {
        report::print_session(&s, Duration::from_millis(s.duration_ms));
    }
    Ok(())
}

fn revert_cmd(a: RevertArgs) -> Result<()> {
    let s = match a.id {
        Some(id) => session::load(&id)?,
        None => session::latest()?,
    };
    revert::run(
        &s,
        &revert::Options {
            yes: a.yes,
            dry_run: a.dry_run,
            kill: a.kill,
        },
    )
}
