use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "aleph", about = "Aleph — Context Store for your desktop", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon + API server (default)
    Start,
    /// Start just the background daemon
    Daemon,
    /// Start just the API server
    Api,
    /// Stop running Aleph (via systemd)
    Stop,
    /// Check if Aleph is running
    Status,
    /// Show recent logs (journalctl)
    Logs,
    /// Diagnose common issues
    Doctor,
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// Print version and exit
    Version,
}

#[derive(Subcommand)]
enum ConfigCommands {
    /// Create default config file
    Init,
    /// Print current config
    Show,
    /// Set a config value: aleph config set <key> <value>
    Set {
        /// Dot-notation key (e.g. general.port, polling.interval_secs)
        key: String,
        /// Value
        value: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Start => start_both().await,
        Commands::Daemon => start_daemon().await,
        Commands::Api => start_api().await,
        Commands::Stop => stop().await,
        Commands::Status => status().await,
        Commands::Logs => logs().await,
        Commands::Doctor => doctor().await,
        Commands::Config { action } => match action {
            ConfigCommands::Init => config_init(),
            ConfigCommands::Show => config_show(),
            ConfigCommands::Set { key, value } => config_set(&key, &value),
        },
        Commands::Version => {
            println!("Aleph {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn start_both() -> anyhow::Result<()> {
    let config = aleph_core::Config::init_global()?;
    aleph_core::Config::init_default()?;

    println!("  ┌─ Aleph ──────────────────────────────┐");
    println!("  │  Context Store for your desktop       │");
    println!("  │  http://localhost:{}                  │", config.general.port);
    println!("  └───────────────────────────────────────┘");

    let daemon_config = config.clone();
    let api_config = config.clone();

    let daemon_handle = tokio::spawn(async move {
        if let Err(e) = aleph_daemon::run_daemon(&daemon_config).await {
            eprintln!("Daemon error: {}", e);
        }
    });

    let api_handle = tokio::spawn(async move {
        if let Err(e) = aleph_api::run_api(&api_config).await {
            eprintln!("API error: {}", e);
        }
    });

    let (daemon_res, api_res) = tokio::join!(daemon_handle, api_handle);

    if let Err(e) = daemon_res {
        eprintln!("Daemon task panicked: {}", e);
    }
    if let Err(e) = api_res {
        eprintln!("API task panicked: {}", e);
    }

    Ok(())
}

async fn start_daemon() -> anyhow::Result<()> {
    let config = aleph_core::Config::init_global()?;
    aleph_core::Config::init_default()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();
    aleph_daemon::run_daemon(config).await
}

async fn start_api() -> anyhow::Result<()> {
    let config = aleph_core::Config::init_global()?;
    aleph_core::Config::init_default()?;
    aleph_api::run_api(config).await
}

async fn stop() -> anyhow::Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(["--user", "stop", "aleph"])
        .status();
    match status {
        Ok(s) if s.success() => println!("Aleph stopped."),
        _ => eprintln!("Could not stop Aleph (systemctl not available or service not found)."),
    }
    Ok(())
}

async fn status() -> anyhow::Result<()> {
    let output = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "aleph"])
        .output();
    match output {
        Ok(out) => {
            let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
            match status.as_str() {
                "active" => println!("Aleph is running."),
                "inactive" => println!("Aleph is not running."),
                _ => println!("Aleph status: {}", status),
            }
        }
        Err(_) => println!("Could not check status (systemctl not available)."),
    }
    Ok(())
}

async fn logs() -> anyhow::Result<()> {
    let status = std::process::Command::new("journalctl")
        .args(["--user", "-u", "aleph", "-n", "50", "--no-pager", "-o", "short"])
        .status();
    match status {
        Ok(_) => {}
        Err(_) => eprintln!("Could not fetch logs (journalctl not available)."),
    }
    Ok(())
}

async fn doctor() -> anyhow::Result<()> {

    println!("Aleph Diagnostics");
    println!("══════════════════");
    println!();

    // 1. Binary
    let bin_path = std::env::current_exe().ok();
    if let Some(p) = &bin_path {
        let meta = std::fs::metadata(p).ok();
        let size = meta.map(|m| m.len() / 1_048_576).unwrap_or(0);
        println!("  ✓ Binary: {} ({} MB)", p.display(), size);
    } else {
        println!("  ✗ Binary: unknown path");
    }

    // 2. Config
    let config_path = aleph_core::Config::config_path();
    if config_path.exists() {
        match aleph_core::Config::load() {
            Ok(cfg) => {
                println!("  ✓ Config: {} (valid)", config_path.display());
                println!("    Port: {}", cfg.general.port);
                println!("    Data dir: {}", cfg.general.data_dir);
                println!("    Log level: {}", cfg.general.log_level);
                println!("    Poll interval: {}s", cfg.polling.interval_secs);
            }
            Err(e) => println!("  ✗ Config: {} (invalid: {})", config_path.display(), e),
        }
    } else {
        println!("  ✗ Config: not found at {}", config_path.display());
    }

    // 3. Data dir
    let cfg = aleph_core::Config::load().unwrap_or_default();
    let data_dir = cfg.data_dir();
    if data_dir.exists() {
        println!("  ✓ Data dir: {}", data_dir.display());
    } else {
        println!("  ✗ Data dir: not found at {}", data_dir.display());
    }

    // 4. Models
    let models_dir = cfg.models_dir();
    let minilm_dir = models_dir.join("all-MiniLM-L6-v2");
    let siglip_dir = models_dir.join("siglip");

    let minilm_ok = minilm_dir.join("model.safetensors").exists() && minilm_dir.join("config.json").exists();
    let siglip_ok = siglip_dir.join("model.safetensors").exists() && siglip_dir.join("config.json").exists();

    if minilm_ok {
        let size = std::fs::metadata(minilm_dir.join("model.safetensors"))
            .map(|m| m.len() / 1_048_576).unwrap_or(0);
        println!("  ✓ MiniLM: {} ({} MB)", minilm_dir.display(), size);
    } else {
        println!("  ✗ MiniLM: missing or incomplete at {}", minilm_dir.display());
    }

    if siglip_ok {
        let size = std::fs::metadata(siglip_dir.join("model.safetensors"))
            .map(|m| m.len() / 1_048_576).unwrap_or(0);
        println!("  ✓ SigLIP: {} ({} MB)", siglip_dir.display(), size);
    } else {
        println!("  ✗ SigLIP: missing or incomplete at {}", siglip_dir.display());
    }

    // 5. Port available
    let port = cfg.general.port;
    let port_available = std::net::TcpListener::bind(format!("127.0.0.1:{}", port)).is_ok();
    if port_available {
        println!("  ✓ Port {}: available", port);
    } else {
        println!("  ⚠ Port {}: already in use (Aleph may already be running)", port);
    }

    // 6. DISPLAY / X11
    let display = std::env::var("DISPLAY").unwrap_or_default();
    if !display.is_empty() {
        println!("  ✓ DISPLAY: {}", display);
    } else {
        println!("  ⚠ DISPLAY: not set (xcap screen capture unavailable)");
    }

    // 7. Systemd service
    let svc_path = aleph_core::Config::config_dir().join("aleph.service");
    // also check the systemd user path
    let svc_path2 = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("systemd/user/aleph.service");
    let svc_exists = svc_path.exists() || svc_path2.exists();
    if svc_exists {
        println!("  ✓ Service file: {}", svc_path.display());
        let active = std::process::Command::new("systemctl")
            .args(["--user", "is-active", "aleph"])
            .output();
        match active {
            Ok(out) => {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if s == "active" {
                    println!("  ✓ Service: active");
                } else {
                    println!("  ⚠ Service: {}", s);
                }
            }
            Err(_) => println!("  ⚠ Service: could not check (systemctl not available)"),
        }
    } else {
        println!("  ✗ Service file: not found at {}", svc_path.display());
    }

    // 8. API reachability
    match std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(_) => println!("  ✓ API: http://127.0.0.1:{}/ accepts connections", port),
        Err(_) => println!("  ✗ API: http://127.0.0.1:{}/ not reachable (is Aleph running?)", port),
    }

    println!();
    Ok(())
}

fn config_init() -> anyhow::Result<()> {
    aleph_core::Config::init_default()?;
    println!("Config created at {:?}", aleph_core::Config::config_path());
    Ok(())
}

fn config_show() -> anyhow::Result<()> {
    let cfg = aleph_core::Config::load()?;
    println!("{}", toml::to_string_pretty(&cfg)?);
    Ok(())
}

fn config_set(key: &str, value: &str) -> anyhow::Result<()> {
    let mut cfg = aleph_core::Config::load()?;

    match key {
        "general.port" => cfg.general.port = value.parse()?,
        "general.log_level" => cfg.general.log_level = value.to_string(),
        "general.data_dir" => cfg.general.data_dir = value.to_string(),
        "polling.interval_secs" => cfg.polling.interval_secs = value.parse()?,
        "dedup.threshold" => cfg.dedup.threshold = value.parse()?,
        "dedup.last_n" => cfg.dedup.last_n = value.parse()?,
        "encoders.text" => cfg.encoders.text = value.parse()?,
        "encoders.vision" => cfg.encoders.vision = value.parse()?,
        "retention.max_events" => cfg.retention.max_events = value.parse()?,
        "dashboard.theme" => cfg.dashboard.theme = value.to_string(),
        _ => anyhow::bail!("Unknown config key: {}. Use dot notation, e.g. general.port", key),
    }

    cfg.save()?;
    println!("{} set to {}", key, value);
    Ok(())
}
