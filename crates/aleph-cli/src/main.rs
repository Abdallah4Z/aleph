use clap::{Parser, Subcommand};

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
