//! Server configuration, from the environment only (never from HTTP).

use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub dlc_dir: PathBuf,
    pub config_dir: PathBuf,
    pub static_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub host: String,
    pub port: u16,
    pub client_ip_header: Option<String>,
    pub rescan_secs: u64,
    pub cache_bytes: u64,
    pub version: String,
    pub source_url: String,
    pub license_url: String,
}

fn env(k: &str) -> Option<String> {
    std::env::var(k)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn http_url(v: Option<String>) -> Option<String> {
    v.filter(|u| u.starts_with("https://") || u.starts_with("http://"))
}

impl Config {
    pub fn from_env() -> Self {
        let source_url = http_url(env("APP_SOURCE_URL"))
            .unwrap_or_else(|| "https://github.com/Imdabossou/RiffBoard".into());
        let license_url = http_url(env("APP_LICENSE_URL"))
            .unwrap_or_else(|| format!("{}/blob/main/LICENSE", source_url.trim_end_matches('/')));
        Self {
            dlc_dir: env("DLC_DIR").map_or_else(|| "/dlc".into(), PathBuf::from),
            config_dir: env("CONFIG_DIR").map_or_else(|| "/config".into(), PathBuf::from),
            static_dir: env("STATIC_DIR").map_or_else(|| "static".into(), PathBuf::from),
            plugins_dir: env("PLUGINS_DIR").map_or_else(|| "plugins".into(), PathBuf::from),
            host: env("HOST").unwrap_or_else(|| "0.0.0.0".into()),
            port: env("PORT").and_then(|p| p.parse().ok()).unwrap_or(8000),
            client_ip_header: env("RIFFBOARD_CLIENT_IP_HEADER").map(|h| h.to_ascii_lowercase()),
            rescan_secs: env("RIFFBOARD_RESCAN_SECONDS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300)
                .max(30),
            cache_bytes: env("RIFFBOARD_CACHE_MB")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2048)
                * 1024
                * 1024,
            version: env("APP_VERSION").unwrap_or_else(|| env!("CARGO_PKG_VERSION").into()),
            source_url,
            license_url,
        }
    }
}
