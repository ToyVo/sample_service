#!/usr/bin/env -S cargo -Zscript
---cargo
[package]
edition = "2021"
---
//! Manage the local observability stack: OTEL collector, Tempo, Prometheus,
//! Grafana. Data and logs live in `./dev.observability/` (gitignored).
//!
//! Usage: `otel up|down|restart|status|logs [service]|clean` (default: `up`).
//! Run via `cgs otel-up` (etc.) or `cargo -Zscript ./scripts/otel.rs <action>`.
//!
//! Endpoints: OTLP http://localhost:4318 (gRPC :4317), Grafana
//! http://localhost:3000 (anonymous admin, Tempo + Prometheus provisioned),
//! Prometheus http://localhost:9090, Tempo http://localhost:3200.
//!
//! Binaries come from PATH (`nix develop` provides them).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

struct Service {
    name: &'static str,
    pid_file: &'static str,
    log_file: &'static str,
    check_ports: &'static [u16],
    timeout_secs: u64,
}

const SERVICES: &[Service] = &[
    Service {
        name: "tempo",
        pid_file: "tempo.pid",
        log_file: "tempo.log",
        check_ports: &[3200],
        timeout_secs: 45,
    },
    Service {
        name: "prometheus",
        pid_file: "prometheus.pid",
        log_file: "prometheus.log",
        check_ports: &[9090],
        timeout_secs: 30,
    },
    Service {
        name: "collector",
        pid_file: "otelcol.pid",
        log_file: "otelcol.log",
        check_ports: &[4317, 4318],
        timeout_secs: 30,
    },
    Service {
        name: "grafana",
        pid_file: "grafana.pid",
        log_file: "grafana.log",
        check_ports: &[3000],
        timeout_secs: 90,
    },
];

const DATA_DIRS: &[&str] = &[
    "prometheus-data",
    "tempo-data",
    "tempo-wal",
    "tempo-live-wal",
    "tempo-block-builder-wal",
    "tempo-work",
    "tempo-shutdown-marker",
    "grafana-data",
    "grafana-logs",
];

fn usage() -> String {
    "Usage: otel up|down|restart|status|logs [collector|tempo|prometheus|grafana]|clean\n\
     \n\
     Manages a local observability stack (data in ./dev.observability/).\n\
     Binaries (otelcol, tempo, prometheus, grafana) come from PATH:\n\
     run inside `nix develop` from the repo root."
        .to_string()
}

/// Repo root: Scripts run from there, but resolve robustly anyway.
fn find_root() -> Result<PathBuf, String> {
    let cwd =
        std::env::current_dir().map_err(|err| format!("cannot read working directory: {err}"))?;
    if cwd.join("otel-collector.yaml").exists() {
        return Ok(cwd);
    }
    Err("cannot locate repo root (otel-collector.yaml not found here)".to_string())
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
        "'{name}' not found in PATH. Run inside `nix develop` from the repo root."
    ))
}

fn obs_dir(root: &Path) -> PathBuf {
    root.join("dev.observability")
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .output()
        .is_ok_and(|output| output.status.success())
}

fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}")
            .parse()
            .expect("loopback addr parses"),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn wait_ports(service: &Service) -> Result<(), String> {
    for _ in 0..service.timeout_secs {
        if service.check_ports.iter().all(|port| port_open(*port)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(format!(
        "{} did not open {:?} in time; see dev.observability/{}",
        service.name, service.check_ports, service.log_file
    ))
}

/// Command line (plus extra env) for a service. Paths are absolute.
fn service_command(
    root: &Path,
    obs: &Path,
    service: &Service,
) -> Result<(PathBuf, Vec<String>, Vec<(String, String)>), String> {
    let arg = |text: &str| text.to_owned();
    match service.name {
        "collector" => {
            let bin = need_bin("otelcol")?;
            let config = root.join("otel-collector.yaml");
            Ok((
                bin,
                vec![arg("--config"), config.to_string_lossy().into_owned()],
                Vec::new(),
            ))
        }
        "tempo" => {
            let bin = need_bin("tempo")?;
            let config = root.join("tempo.yaml");
            // Keep all Tempo state under ./dev.observability (defaults point
            // at /var/tempo, which is not writable for local dev).
            Ok((
                bin,
                vec![
                    arg("--config.file"),
                    config.to_string_lossy().into_owned(),
                    arg("--live-store.shutdown_marker_dir"),
                    obs.join("tempo-shutdown-marker")
                        .to_string_lossy()
                        .into_owned(),
                    arg("--live-store.wal.path"),
                    obs.join("tempo-live-wal").to_string_lossy().into_owned(),
                    arg("--block-builder.wal.path"),
                    obs.join("tempo-block-builder-wal")
                        .to_string_lossy()
                        .into_owned(),
                    arg("-backend-schedulerbackend-scheduler.local-work-path"),
                    obs.join("tempo-work").to_string_lossy().into_owned(),
                ],
                Vec::new(),
            ))
        }
        "prometheus" => {
            let bin = need_bin("prometheus")?;
            let config = root.join("prometheus.yaml");
            Ok((
                bin,
                vec![
                    arg("--config.file"),
                    config.to_string_lossy().into_owned(),
                    "--storage.tsdb.path".to_owned(),
                    obs.join("prometheus-data").to_string_lossy().into_owned(),
                    "--web.listen-address=127.0.0.1:9090".to_owned(),
                ],
                Vec::new(),
            ))
        }
        "grafana" => {
            let bin = need_bin("grafana")?;
            let home = bin
                .parent()
                .and_then(|dir| dir.parent())
                .map(|prefix| prefix.join("share/grafana"))
                .filter(|dir| dir.is_dir())
                .ok_or_else(|| {
                    "cannot locate grafana share dir next to the grafana binary".to_string()
                })?;
            let config = root.join("grafana.ini");
            let env = vec![
                (
                    "GF_PATHS_DATA".to_owned(),
                    obs.join("grafana-data").to_string_lossy().into_owned(),
                ),
                (
                    "GF_PATHS_LOGS".to_owned(),
                    obs.join("grafana-logs").to_string_lossy().into_owned(),
                ),
                (
                    "GF_PATHS_PROVISIONING".to_owned(),
                    root.join("grafana/provisioning")
                        .to_string_lossy()
                        .into_owned(),
                ),
            ];
            Ok((
                bin,
                vec![
                    arg("server"),
                    arg("--homepath"),
                    home.to_string_lossy().into_owned(),
                    arg("--config"),
                    config.to_string_lossy().into_owned(),
                ],
                env,
            ))
        }
        other => Err(format!("unknown service '{other}'")),
    }
}

fn find_service(name: &str) -> Result<&'static Service, String> {
    SERVICES
        .iter()
        .find(|service| service.name == name)
        .ok_or_else(|| format!("unknown service '{name}'; pick collector|tempo|prometheus|grafana"))
}

fn start_one(root: &Path, obs: &Path, service: &Service) -> Result<(), String> {
    let pid_path = obs.join(service.pid_file);
    if let Some(pid) = read_pid(&pid_path) {
        if pid_alive(pid) {
            println!("{} already running (pid {pid}).", service.name);
            return wait_ports(service);
        }
    }
    println!("Starting {}...", service.name);
    let (bin, args, env) = service_command(root, obs, service)?;
    let log = std::fs::File::create(obs.join(service.log_file))
        .map_err(|err| format!("cannot open log: {err}"))?;
    let child = Command::new(&bin)
        .args(&args)
        .envs(env)
        .current_dir(root)
        .stdout(log.try_clone().map_err(|err| format!("log setup: {err}"))?)
        .stderr(log)
        .spawn()
        .map_err(|err| format!("failed to start {}: {err}", bin.display()))?;
    std::fs::write(&pid_path, child.id().to_string())
        .map_err(|err| format!("cannot write pidfile: {err}"))?;
    // Detach: the child outlives this manager (logs/pidfile track it).
    std::mem::forget(child);
    wait_ports(service)
}

fn stop_one(obs: &Path, service: &Service) -> Result<(), String> {
    let pid_path = obs.join(service.pid_file);
    let Some(pid) = read_pid(&pid_path) else {
        println!("{} not running (no pidfile).", service.name);
        return Ok(());
    };
    if !pid_alive(pid) {
        println!("{} already stopped (stale pidfile).", service.name);
        let _ = std::fs::remove_file(&pid_path);
        return Ok(());
    }
    println!("Stopping {} (pid {pid})...", service.name);
    let kill = need_bin("kill")?;
    Command::new(&kill)
        .arg(pid.to_string())
        .output()
        .map_err(|err| format!("failed to signal {pid}: {err}"))?;
    for _ in 0..15 {
        if !pid_alive(pid) {
            let _ = std::fs::remove_file(&pid_path);
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(format!("{pid} did not exit; kill it manually",))
}

fn cmd_up(root: &Path) -> Result<(), String> {
    let obs = obs_dir(root);
    std::fs::create_dir_all(&obs).map_err(|err| format!("cannot create {obs:?}: {err}"))?;
    for dir in DATA_DIRS {
        std::fs::create_dir_all(obs.join(dir))
            .map_err(|err| format!("cannot create data dir: {err}"))?;
    }
    for service in SERVICES {
        start_one(root, &obs, service)?;
    }
    // Post-flight: a service can open its port and still crash during boot
    // (seen with Tempo module failures), so re-verify everything is up.
    for service in SERVICES {
        if !service.check_ports.iter().all(|port| port_open(*port)) {
            return Err(format!(
                "{} died after starting; see dev.observability/{}",
                service.name, service.log_file
            ));
        }
    }
    println!();
    println!("Observability stack ready:");
    println!("  Grafana (Tempo + Prometheus): http://localhost:3000");
    println!("  Prometheus:                   http://localhost:9090");
    println!("  Tempo:                        http://localhost:3200");
    println!("  OTLP HTTP:                    http://localhost:4318 (gRPC :4317)");
    println!("Point the app at it (default) and run `cgs dev-sqlite`.");
    Ok(())
}

fn cmd_down(root: &Path) -> Result<(), String> {
    let obs = obs_dir(root);
    // Reverse start order.
    let mut order: Vec<&Service> = SERVICES.iter().collect();
    order.reverse();
    for service in order {
        stop_one(&obs, service)?;
    }
    Ok(())
}

fn cmd_status(root: &Path) -> Result<(), String> {
    let obs = obs_dir(root);
    for service in SERVICES {
        let pid_text = read_pid(&obs.join(service.pid_file))
            .map(|pid| {
                if pid_alive(pid) {
                    format!("pid {pid}")
                } else {
                    "stale pidfile".to_owned()
                }
            })
            .unwrap_or_else(|| "not running".to_owned());
        let ports: Vec<String> = service
            .check_ports
            .iter()
            .map(|port| {
                if port_open(*port) {
                    format!(":{port} open")
                } else {
                    format!(":{port} closed")
                }
            })
            .collect();
        println!("{:>10}  {pid_text:<16} {}", service.name, ports.join(" "));
    }
    println!("App OTLP default is http://localhost:4318 (see OTEL_EXPORTER_OTLP_ENDPOINT).");
    Ok(())
}

fn cmd_logs(root: &Path, service_name: &str) -> Result<(), String> {
    let service = find_service(service_name)?;
    let log = obs_dir(root).join(service.log_file);
    if !log.is_file() {
        return Err(format!("no log yet at {} (run up first)", log.display()));
    }
    let tail = need_bin("tail")?;
    let status = Command::new(&tail)
        .arg("-f")
        .arg(&log)
        .status()
        .map_err(|err| format!("failed to run tail: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("tail exited unexpectedly".to_string())
    }
}

fn cmd_clean(root: &Path) -> Result<(), String> {
    let _ = cmd_down(root);
    let obs = obs_dir(root);
    if obs.exists() {
        println!("Removing {}...", obs.display());
        std::fs::remove_dir_all(&obs)
            .map_err(|err| format!("cannot remove {}: {err}", obs.display()))?;
    }
    println!("Clean. Next up re-creates everything fresh.");
    Ok(())
}

fn real_main(action: &str, extra: Option<&str>) -> Result<(), String> {
    let root = find_root()?;
    match action {
        "up" => cmd_up(&root),
        "down" => cmd_down(&root),
        "restart" => {
            let _ = cmd_down(&root);
            cmd_up(&root)
        }
        "status" => cmd_status(&root),
        "logs" => cmd_logs(&root, extra.unwrap_or("collector")),
        "clean" => cmd_clean(&root),
        "help" | "-h" | "--help" => {
            println!("{}", usage());
            Ok(())
        }
        other => Err(format!("unknown action '{other}'.\n\n{}", usage())),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let action = args.next();
    let extra = args.next();
    match real_main(action.as_deref().unwrap_or("up"), extra.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
