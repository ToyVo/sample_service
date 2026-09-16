#!/usr/bin/env -S cargo -Zscript
---cargo
[package]
edition = "2024"
---
//! Manage the local Postgres development database.
//!
//! Data lives in `./dev.postgresql/` (gitignored, next to `./dev.db`).
//! Port 5433 avoids clashing with any system Postgres, the Unix socket lives
//! inside the data dir, and trust auth means no password for local dev:
//!
//! ```text
//! DATABASE_URL=postgresql://postgres@localhost:5433/sample
//! ```
//!
//! Usage: `pg-manage start|stop|restart|status|logs|clean` (default: `start`).
//!
//! Built with the system toolchain only (no crates, no network):
//!
//! ```sh
//! rustc --edition=2024 -O -o target/pg-manage scripts/postgres.rs
//! ```
//!
//! See `Scripts.toml` (`postgres-build`, `postgres-start`, …), which builds
//! before every run so the binary never goes stale.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const PORT: &str = "5433";
const SUPERUSER: &str = "postgres";
const DB_NAME: &str = "sample";

fn usage() -> String {
    "Usage: pg-manage start|stop|restart|status|logs|clean\n\
     \n\
     Manages a local Postgres (data in ./dev.postgresql/, port 5433).\n\
     Postgres binaries come from PATH (use `nix develop` from the repo root)."
        .to_string()
}

fn database_url() -> String {
    format!("postgresql://{SUPERUSER}@localhost:{PORT}/{DB_NAME}")
}

/// Locate the repo root: `<root>/target/<binary>` first, working directory
/// as fallback (`cgs` runs from the repo root).
fn find_root() -> Result<PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(target) = exe.parent() {
            if let Some(root) = target.parent() {
                if root.join("diesel.sqlite.toml").exists() {
                    return Ok(root.to_path_buf());
                }
            }
        }
    }
    let cwd =
        std::env::current_dir().map_err(|err| format!("cannot read working directory: {err}"))?;
    if cwd.join("diesel.sqlite.toml").exists() {
        return Ok(cwd);
    }
    Err(
        "cannot locate repo root (looked relative to the binary and the working directory)"
            .to_string(),
    )
}

fn need_bin(name: &str) -> Result<PathBuf, String> {
    let paths = std::env::var_os("PATH").ok_or_else(|| "PATH is not set".to_string())?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "'{name}' not found in PATH. Run inside `nix develop` from the repo root, \
         which provides postgresql, or install Postgres 16+ another way."
    ))
}

/// Run a binary, returning stdout on success or trimmed stderr on failure.
fn run(bin: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|err| format!("failed to run {}: {err}", bin.display()))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(format!(
            "{} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

struct Ctx {
    data_dir: PathBuf,
    log_file: PathBuf,
    pg_ctl: PathBuf,
    pg_isready: PathBuf,
    psql: PathBuf,
    createdb: PathBuf,
    initdb: PathBuf,
}

impl Ctx {
    fn load() -> Result<Self, String> {
        let root = find_root()?;
        let data_dir = root.join("dev.postgresql");
        let log_file = data_dir.join("logfile");
        Ok(Self {
            data_dir,
            log_file,
            pg_ctl: need_bin("pg_ctl")?,
            pg_isready: need_bin("pg_isready")?,
            psql: need_bin("psql")?,
            createdb: need_bin("createdb")?,
            initdb: need_bin("initdb")?,
        })
    }

    fn initialized(&self) -> bool {
        self.data_dir.join("PG_VERSION").is_file()
    }

    fn running(&self) -> bool {
        Command::new(&self.pg_ctl)
            .arg("-D")
            .arg(&self.data_dir)
            .arg("status")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn ready(&self) -> bool {
        Command::new(&self.pg_isready)
            .arg("-h")
            .arg("localhost")
            .arg("-p")
            .arg(PORT)
            .arg("-q")
            .output()
            .is_ok_and(|output| output.status.success())
    }
}

fn cmd_init(ctx: &Ctx) -> Result<(), String> {
    if ctx.initialized() {
        return Ok(());
    }
    println!("Initializing Postgres data dir: {}", ctx.data_dir.display());
    std::fs::create_dir_all(&ctx.data_dir)
        .map_err(|err| format!("cannot create {}: {err}", ctx.data_dir.display()))?;
    // Trust local connections so no password is needed for dev.
    run(
        &ctx.initdb,
        &[
            "-D",
            &ctx.data_dir.to_string_lossy(),
            "-U",
            SUPERUSER,
            "--auth=trust",
            "-E",
            "UTF8",
            "--no-locale",
        ],
    )?;
    Ok(())
}

fn wait_ready(ctx: &Ctx) -> Result<(), String> {
    for _ in 0..60 {
        if ctx.ready() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!(
        "Postgres did not become ready in time. See {}",
        ctx.log_file.display()
    ))
}

fn ensure_db(ctx: &Ctx) -> Result<(), String> {
    let existing = run(
        &ctx.psql,
        &[
            "-h",
            "localhost",
            "-p",
            PORT,
            "-U",
            SUPERUSER,
            "-d",
            "postgres",
            "-tAc",
            &format!("SELECT 1 FROM pg_database WHERE datname='{DB_NAME}'"),
        ],
    )?;
    if existing.trim() == "1" {
        return Ok(());
    }
    println!("Creating database '{DB_NAME}'...");
    run(
        &ctx.createdb,
        &["-h", "localhost", "-p", PORT, "-U", SUPERUSER, DB_NAME],
    )?;
    Ok(())
}

fn cmd_start(ctx: &Ctx) -> Result<(), String> {
    cmd_init(ctx)?;
    if ctx.running() {
        println!(
            "Postgres already running (data dir {}).",
            ctx.data_dir.display()
        );
    } else {
        println!(
            "Starting Postgres (data dir {}, port {PORT})...",
            ctx.data_dir.display()
        );
        // `-p` overrides the port from postgresql.conf; `-k` puts the Unix
        // socket inside the data dir so it cannot clash with system Postgres.
        let socket_opt = format!("-p {PORT} -k \"{}\"", ctx.data_dir.display());
        run(
            &ctx.pg_ctl,
            &[
                "-D",
                &ctx.data_dir.to_string_lossy(),
                "-l",
                &ctx.log_file.to_string_lossy(),
                "-o",
                &socket_opt,
                "start",
            ],
        )?;
    }
    wait_ready(ctx)?;
    ensure_db(ctx)?;
    println!("Postgres ready.");
    println!("DATABASE_URL={}", database_url());
    Ok(())
}

fn cmd_stop(ctx: &Ctx) -> Result<(), String> {
    if !ctx.initialized() {
        println!(
            "Nothing to stop (no data dir at {}).",
            ctx.data_dir.display()
        );
        return Ok(());
    }
    if ctx.running() {
        println!("Stopping Postgres...");
        run(
            &ctx.pg_ctl,
            &["-D", &ctx.data_dir.to_string_lossy(), "stop", "-m", "fast"],
        )?;
    } else {
        println!("Postgres already stopped.");
    }
    Ok(())
}

fn cmd_status(ctx: &Ctx) -> Result<(), String> {
    if !ctx.initialized() {
        return Err(format!(
            "Not initialized (no {} yet — run 'start').",
            ctx.data_dir.display()
        ));
    }
    // Report both checks without failing: an exit code of 0 means the script
    // ran, not that the server is up (mirrors `pg_ctl status` usage).
    let status = run(
        &ctx.pg_ctl,
        &["-D", &ctx.data_dir.to_string_lossy(), "status"],
    );
    match status {
        Ok(out) => print!("{out}"),
        Err(err) => println!("{err}"),
    }
    if ctx.ready() {
        println!("localhost:{PORT} - accepting connections");
    } else {
        println!("localhost:{PORT} - no response");
    }
    println!("DATABASE_URL={}", database_url());
    Ok(())
}

fn cmd_logs(ctx: &Ctx) -> Result<(), String> {
    if !ctx.log_file.is_file() {
        return Err(format!(
            "No logfile yet at {} (run 'start' first).",
            ctx.log_file.display()
        ));
    }
    let tail = need_bin("tail")?;
    let status = Command::new(&tail)
        .arg("-f")
        .arg(&ctx.log_file)
        .status()
        .map_err(|err| format!("failed to run tail: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("tail exited unexpectedly".to_string())
    }
}

fn cmd_clean(ctx: &Ctx) -> Result<(), String> {
    // Best effort: a missing server is not an error when cleaning.
    let _ = cmd_stop(ctx);
    if ctx.data_dir.exists() {
        println!("Removing {}...", ctx.data_dir.display());
        std::fs::remove_dir_all(&ctx.data_dir)
            .map_err(|err| format!("cannot remove {}: {err}", ctx.data_dir.display()))?;
    }
    println!("Clean. Next 'start' will re-init a fresh database.");
    Ok(())
}

fn real_main(action: &str) -> Result<(), String> {
    let ctx = Ctx::load()?;
    match action {
        "start" => cmd_start(&ctx),
        "stop" => cmd_stop(&ctx),
        "restart" => {
            let _ = cmd_stop(&ctx);
            cmd_start(&ctx)
        }
        "status" => cmd_status(&ctx),
        "logs" => cmd_logs(&ctx),
        "clean" => cmd_clean(&ctx),
        "help" | "-h" | "--help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown action '{other}'.\n\n{}", usage())),
    }
}

fn main() -> ExitCode {
    let action = std::env::args().nth(1);
    match real_main(action.as_deref().unwrap_or("start")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
