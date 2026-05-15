use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;

static CONFIG: OnceLock<Config> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub polling: PollingConfig,
    pub dedup: DedupConfig,
    pub encoders: EncodersConfig,
    pub retention: RetentionConfig,
    pub dashboard: DashboardConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub data_dir: String,
    pub port: u16,
    pub log_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollingConfig {
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DedupConfig {
    pub threshold: f32,
    pub last_n: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncodersConfig {
    pub text: bool,
    pub vision: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    pub max_events: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig {
                data_dir: dirs::data_dir()
                    .unwrap_or_else(|| PathBuf::from("~/.local/share"))
                    .join("aleph")
                    .to_string_lossy()
                    .to_string(),
                port: 2198,
                log_level: "info".into(),
            },
            polling: PollingConfig { interval_secs: 2 },
            dedup: DedupConfig {
                threshold: 0.95,
                last_n: 5,
            },
            encoders: EncodersConfig {
                text: true,
                vision: true,
            },
            retention: RetentionConfig { max_events: 10000 },
            dashboard: DashboardConfig {
                theme: "dark".into(),
            },
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("aleph")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn data_dir(&self) -> PathBuf {
        let p = shellexpand(&self.general.data_dir);
        std::fs::create_dir_all(&p).ok();
        p
    }

    pub fn models_dir(&self) -> PathBuf {
        self.data_dir().join("models")
    }

    /// Load config from ~/.config/aleph/config.toml, merge with env overrides.
    pub fn load() -> Result<Self> {
        let mut cfg = Config::default();

        let config_path = Self::config_path();
        if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)?;
            let file_cfg: Config = toml::from_str(&raw)?;
            cfg.merge(file_cfg);
        }

        cfg.apply_env_overrides();
        Ok(cfg)
    }

    /// Load once and cache globally.
    pub fn global() -> &'static Config {
        CONFIG.get_or_init(|| {
            Config::load().unwrap_or_default()
        })
    }

    /// Initialize global config (call at startup).
    pub fn init_global() -> Result<&'static Config> {
        let cfg = Config::load()?;
        CONFIG.set(cfg).ok();
        Ok(Config::global())
    }

    /// Save current config to file, creating parent dirs.
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let raw = toml::to_string_pretty(self)?;
        std::fs::write(&path, raw)?;
        Ok(())
    }

    /// Create default config file if it doesn't exist.
    pub fn init_default() -> Result<()> {
        let path = Self::config_path();
        if !path.exists() {
            Self::default().save()?;
            tracing::info!("Default config created at {:?}", path);
        }
        Ok(())
    }

    fn merge(&mut self, other: Config) {
        if other.general.data_dir != Config::default().general.data_dir {
            self.general.data_dir = other.general.data_dir;
        }
        if other.general.port != Config::default().general.port {
            self.general.port = other.general.port;
        }
        if other.general.log_level != Config::default().general.log_level {
            self.general.log_level = other.general.log_level;
        }
        if other.polling.interval_secs != Config::default().polling.interval_secs {
            self.polling.interval_secs = other.polling.interval_secs;
        }
        if (other.dedup.threshold - Config::default().dedup.threshold).abs() > f32::EPSILON {
            self.dedup.threshold = other.dedup.threshold;
        }
        if other.dedup.last_n != Config::default().dedup.last_n {
            self.dedup.last_n = other.dedup.last_n;
        }
        if other.retention.max_events != Config::default().retention.max_events {
            self.retention.max_events = other.retention.max_events;
        }
        self.encoders.text = other.encoders.text;
        self.encoders.vision = other.encoders.vision;
        if other.dashboard.theme != Config::default().dashboard.theme {
            self.dashboard.theme = other.dashboard.theme;
        }
    }

    fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("ALEPH_PORT") {
            if let Ok(p) = v.parse() {
                self.general.port = p;
            }
        }
        if let Ok(v) = std::env::var("ALEPH_LOG_LEVEL") {
            self.general.log_level = v;
        }
        if let Ok(v) = std::env::var("ALEPH_DATA_DIR") {
            self.general.data_dir = v;
        }
        if let Ok(v) = std::env::var("ALEPH_POLLING_INTERVAL") {
            if let Ok(p) = v.parse() {
                self.polling.interval_secs = p;
            }
        }
        if let Ok(v) = std::env::var("ALEPH_DEDUP_THRESHOLD") {
            if let Ok(p) = v.parse() {
                self.dedup.threshold = p;
            }
        }
    }
}

fn shellexpand(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~") {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        PathBuf::from(home).join(rest.strip_prefix('/').unwrap_or(rest))
    } else {
        PathBuf::from(s)
    }
}
