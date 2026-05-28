use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

pub mod clash;
pub mod controller;

pub const CHATGPT_URL: &str = "https://chatgpt.com";
pub const APP_DIR_NAME: &str = "ChatGPTWebviewClient";
pub const WEBVIEW_PROFILE_DIR_NAME: &str = "WebView2Profile";
pub const PORTABLE_DATA_DIR_NAME: &str = "data";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyScheme {
    Http,
    Socks5,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxySettings {
    pub scheme: ProxyScheme,
    pub host: String,
    pub port: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    System,
    Direct,
    InternalClash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyUiSettings {
    pub mode: ProxyMode,
    pub subscription_url: String,
    #[serde(default)]
    pub active_subscription_id: String,
    #[serde(default)]
    pub subscriptions: Vec<ProxySubscription>,
    pub mixed_port: u16,
    pub controller_port: u16,
    #[serde(default = "default_auto_update_subscription")]
    pub auto_update_subscription: bool,
    #[serde(default)]
    pub selected_group: String,
    #[serde(default)]
    pub selected_proxy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxySubscription {
    pub id: String,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    pub proxy: ProxyUiSettings,
}

impl AppSettings {
    pub fn normalized(mut self) -> Self {
        self.proxy.normalize_subscriptions();
        self
    }
}

impl ProxyUiSettings {
    pub fn active_subscription_url(&self) -> &str {
        self.subscriptions
            .iter()
            .find(|subscription| subscription.id == self.active_subscription_id)
            .map(|subscription| subscription.url.as_str())
            .filter(|url| !url.trim().is_empty())
            .unwrap_or(&self.subscription_url)
    }

    pub fn normalize_subscriptions(&mut self) {
        self.normalize_subscriptions_with_previous_active(None);
    }

    pub fn normalize_subscriptions_with_previous_active(&mut self, previous_active: Option<&str>) {
        self.subscriptions
            .retain(|subscription| !subscription.url.trim().is_empty());

        if self.subscriptions.is_empty() && !self.subscription_url.trim().is_empty() {
            self.subscriptions.push(ProxySubscription {
                id: stable_subscription_id(&self.subscription_url),
                name: "默认订阅".to_string(),
                url: self.subscription_url.clone(),
            });
        }

        if self.active_subscription_id.trim().is_empty()
            || !self
                .subscriptions
                .iter()
                .any(|subscription| subscription.id == self.active_subscription_id)
        {
            self.active_subscription_id = self
                .subscriptions
                .first()
                .map(|subscription| subscription.id.clone())
                .unwrap_or_default();
        }

        if let Some(active_url) = self
            .subscriptions
            .iter()
            .find(|subscription| subscription.id == self.active_subscription_id)
            .map(|subscription| subscription.url.clone())
        {
            self.subscription_url = active_url;
        }

        if previous_active.is_some_and(|previous| previous != self.active_subscription_id) {
            self.selected_group.clear();
            self.selected_proxy.clear();
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            proxy: ProxyUiSettings {
                mode: ProxyMode::System,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port: 17898,
                controller_port: 17899,
                auto_update_subscription: true,
                selected_group: String::new(),
                selected_proxy: String::new(),
            },
        }
    }
}

fn default_auto_update_subscription() -> bool {
    true
}

pub fn stable_subscription_id(url: &str) -> String {
    let hash = url.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("sub-{hash:016x}")
}

pub fn webview_profile_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join(WEBVIEW_PROFILE_DIR_NAME))
}

pub fn app_data_dir() -> Result<PathBuf> {
    portable_data_dir()
}

pub fn portable_data_dir() -> Result<PathBuf> {
    Ok(executable_dir()?.join(PORTABLE_DATA_DIR_NAME))
}

fn legacy_app_data_dir() -> Result<PathBuf> {
    let base_dirs =
        BaseDirs::new().context("could not resolve the current user's home directory")?;

    Ok(base_dirs.data_local_dir().join(APP_DIR_NAME))
}

fn legacy_webview_profile_dir() -> Result<PathBuf> {
    Ok(legacy_app_data_dir()?.join(WEBVIEW_PROFILE_DIR_NAME))
}

fn executable_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("could not locate current executable")?;
    exe.parent()
        .map(PathBuf::from)
        .context("could not locate current executable directory")
}

pub fn settings_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("settings.toml"))
}

pub fn load_settings() -> AppSettings {
    let _ = migrate_legacy_data();
    let Ok(path) = settings_path() else {
        return AppSettings::default();
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return AppSettings::default();
    };

    toml::from_str::<AppSettings>(&contents)
        .map(AppSettings::normalized)
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> Result<()> {
    migrate_legacy_data()?;
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create application settings directory at {}",
                parent.display()
            )
        })?;
    }

    let settings = settings.clone().normalized();
    let contents = toml::to_string_pretty(&settings).context("could not serialize settings")?;
    std::fs::write(&path, contents)
        .with_context(|| format!("could not write settings to {}", path.display()))
}

pub fn startup_proxy(settings: &AppSettings) -> Option<ProxySettings> {
    match settings.proxy.mode {
        ProxyMode::Direct => None,
        ProxyMode::System => detected_proxy(),
        ProxyMode::InternalClash => Some(ProxySettings {
            scheme: ProxyScheme::Http,
            host: "127.0.0.1".to_string(),
            port: settings.proxy.mixed_port.to_string(),
        }),
    }
}

pub fn parse_proxy_server(value: &str) -> Option<ProxySettings> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if value.contains('=') {
        let entries = value.split(';').filter_map(|entry| entry.split_once('='));

        for preferred_key in ["http", "https", "socks", "socks5"] {
            for (key, endpoint) in entries.clone() {
                if key.trim().eq_ignore_ascii_case(preferred_key) {
                    return parse_proxy_endpoint(endpoint, scheme_for_key(preferred_key));
                }
            }
        }

        return None;
    }

    parse_proxy_endpoint(value, ProxyScheme::Http)
}

pub fn detected_proxy() -> Option<ProxySettings> {
    std::env::var("CHATGPT_CLIENT_PROXY")
        .ok()
        .and_then(|value| parse_proxy_server(&value))
        .or_else(windows_system_proxy)
}

#[cfg(windows)]
pub fn windows_system_proxy() -> Option<ProxySettings> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let internet_settings = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
        .ok()?;
    let enabled: u32 = internet_settings.get_value("ProxyEnable").ok()?;
    if enabled == 0 {
        return None;
    }

    let proxy_server: String = internet_settings.get_value("ProxyServer").ok()?;
    parse_proxy_server(&proxy_server)
}

#[cfg(not(windows))]
pub fn windows_system_proxy() -> Option<ProxySettings> {
    None
}

fn parse_proxy_endpoint(value: &str, fallback_scheme: ProxyScheme) -> Option<ProxySettings> {
    let endpoint = value.trim();
    if endpoint.is_empty() {
        return None;
    }

    let (scheme, endpoint) = if let Some(rest) = endpoint.strip_prefix("http://") {
        (ProxyScheme::Http, rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        (ProxyScheme::Http, rest)
    } else if let Some(rest) = endpoint.strip_prefix("socks5://") {
        (ProxyScheme::Socks5, rest)
    } else if let Some(rest) = endpoint.strip_prefix("socks://") {
        (ProxyScheme::Socks5, rest)
    } else {
        (fallback_scheme, endpoint)
    };

    let endpoint = endpoint.trim_end_matches('/');
    let (host, port) = endpoint.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() {
        return None;
    }

    Some(ProxySettings {
        scheme,
        host: host.trim_matches(['[', ']']).to_string(),
        port: port.to_string(),
    })
}

fn scheme_for_key(key: &str) -> ProxyScheme {
    if key.eq_ignore_ascii_case("socks") || key.eq_ignore_ascii_case("socks5") {
        ProxyScheme::Socks5
    } else {
        ProxyScheme::Http
    }
}

pub fn ensure_webview_profile_dir() -> Result<PathBuf> {
    migrate_legacy_data()?;
    let profile_dir = webview_profile_dir()?;
    std::fs::create_dir_all(&profile_dir).with_context(|| {
        format!(
            "could not create WebView2 profile directory at {}",
            profile_dir.display()
        )
    })?;
    Ok(profile_dir)
}

pub fn ensure_portable_data_dir() -> Result<PathBuf> {
    migrate_legacy_data()?;
    let data_dir = app_data_dir()?;
    std::fs::create_dir_all(&data_dir).with_context(|| {
        format!(
            "could not create portable data directory at {}",
            data_dir.display()
        )
    })?;
    Ok(data_dir)
}

pub fn migrate_legacy_data() -> Result<()> {
    let data_dir = portable_data_dir()?;
    std::fs::create_dir_all(&data_dir).with_context(|| {
        format!(
            "could not create portable data directory at {}",
            data_dir.display()
        )
    })?;

    copy_legacy_file_if_missing(
        &legacy_app_data_dir()?.join("settings.toml"),
        &settings_path()?,
    )?;
    copy_legacy_dir_if_missing(&legacy_webview_profile_dir()?, &webview_profile_dir()?)?;
    copy_legacy_dir_if_missing(
        &legacy_app_data_dir()?.join("clash"),
        &app_data_dir()?.join("clash"),
    )?;

    Ok(())
}

fn copy_legacy_file_if_missing(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
    if target.exists() || !source.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create portable data directory at {}",
                parent.display()
            )
        })?;
    }
    std::fs::copy(source, target).with_context(|| {
        format!(
            "could not migrate legacy file from {} to {}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn copy_legacy_dir_if_missing(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
    if target.exists() || !source.exists() {
        return Ok(());
    }
    copy_dir_recursive(source, target)
}

fn copy_dir_recursive(source: &std::path::Path, target: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(target)
        .with_context(|| format!("could not create directory {}", target.display()))?;

    for entry in std::fs::read_dir(source)
        .with_context(|| format!("could not read directory {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "could not copy {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dir_uses_app_specific_webview_folder() {
        let profile_dir = webview_profile_dir().expect("profile dir should resolve");
        let rendered = profile_dir.to_string_lossy().replace('\\', "/");

        assert!(rendered.ends_with("data/WebView2Profile"));
    }

    #[test]
    fn settings_path_uses_program_data_directory() {
        let path = settings_path().expect("settings path should resolve");
        let rendered = path.to_string_lossy().replace('\\', "/");

        assert!(rendered.ends_with("data/settings.toml"));
    }

    #[test]
    fn app_data_dir_uses_program_data_directory() {
        let path = app_data_dir().expect("app data path should resolve");
        let rendered = path.to_string_lossy().replace('\\', "/");

        assert!(rendered.ends_with("data"));
    }

    #[test]
    fn clash_runtime_dir_is_inside_program_data_directory() {
        let path = app_data_dir()
            .expect("app data path should resolve")
            .join("clash");
        let rendered = path.to_string_lossy().replace('\\', "/");

        assert!(rendered.ends_with("data/clash"));
    }

    #[test]
    fn parses_windows_split_http_proxy_server() {
        let proxy = parse_proxy_server("http=127.0.0.1:7898;https=127.0.0.1:7898")
            .expect("proxy should parse");

        assert_eq!(
            proxy,
            ProxySettings {
                scheme: ProxyScheme::Http,
                host: "127.0.0.1".to_string(),
                port: "7898".to_string(),
            }
        );
    }

    #[test]
    fn parses_plain_proxy_server() {
        let proxy = parse_proxy_server("127.0.0.1:7898").expect("proxy should parse");

        assert_eq!(
            proxy,
            ProxySettings {
                scheme: ProxyScheme::Http,
                host: "127.0.0.1".to_string(),
                port: "7898".to_string(),
            }
        );
    }

    #[test]
    fn parses_socks_proxy_server() {
        let proxy = parse_proxy_server("socks=127.0.0.1:7897").expect("proxy should parse");

        assert_eq!(
            proxy,
            ProxySettings {
                scheme: ProxyScheme::Socks5,
                host: "127.0.0.1".to_string(),
                port: "7897".to_string(),
            }
        );
    }

    #[test]
    fn default_app_settings_use_system_proxy() {
        let settings = AppSettings::default();

        assert_eq!(settings.proxy.mode, ProxyMode::System);
        assert_eq!(settings.proxy.mixed_port, 17898);
        assert_eq!(settings.proxy.controller_port, 17899);
        assert!(settings.proxy.auto_update_subscription);
        assert!(settings.proxy.selected_group.is_empty());
        assert!(settings.proxy.selected_proxy.is_empty());
    }

    #[test]
    fn app_settings_round_trip_through_toml() {
        let settings = AppSettings {
            proxy: ProxyUiSettings {
                mode: ProxyMode::InternalClash,
                subscription_url: "https://example.com/sub".to_string(),
                active_subscription_id: "sub-example".to_string(),
                subscriptions: vec![ProxySubscription {
                    id: "sub-example".to_string(),
                    name: "Example".to_string(),
                    url: "https://example.com/sub".to_string(),
                }],
                mixed_port: 18080,
                controller_port: 18081,
                auto_update_subscription: true,
                selected_group: "PROXY".to_string(),
                selected_proxy: "AUTO".to_string(),
            },
        };

        let rendered = toml::to_string_pretty(&settings).expect("settings should serialize");
        let parsed: AppSettings = toml::from_str(&rendered).expect("settings should parse");

        assert_eq!(parsed, settings);
    }

    #[test]
    fn legacy_subscription_url_migrates_to_subscription_list() {
        let settings: AppSettings = toml::from_str(
            r#"
[proxy]
mode = "internal_clash"
subscription_url = "https://example.com/legacy"
mixed_port = 17898
controller_port = 17899
auto_update_subscription = true
selected_group = "Proxy"
selected_proxy = "Node"
"#,
        )
        .expect("legacy settings should parse");
        let settings = settings.normalized();

        assert_eq!(
            settings.proxy.active_subscription_url(),
            "https://example.com/legacy"
        );
        assert_eq!(settings.proxy.subscriptions.len(), 1);
        assert_eq!(settings.proxy.subscriptions[0].name, "默认订阅");
        assert!(!settings.proxy.active_subscription_id.is_empty());
    }

    #[test]
    fn active_subscription_url_uses_selected_subscription() {
        let mut settings = AppSettings::default();
        settings.proxy.subscription_url = "https://example.com/old".to_string();
        settings.proxy.subscriptions = vec![
            ProxySubscription {
                id: "a".to_string(),
                name: "A".to_string(),
                url: "https://example.com/a".to_string(),
            },
            ProxySubscription {
                id: "b".to_string(),
                name: "B".to_string(),
                url: "https://example.com/b".to_string(),
            },
        ];
        settings.proxy.active_subscription_id = "b".to_string();
        settings.proxy.normalize_subscriptions();

        assert_eq!(
            settings.proxy.active_subscription_url(),
            "https://example.com/b"
        );
        assert_eq!(settings.proxy.subscription_url, "https://example.com/b");
    }

    #[test]
    fn normalizing_subscription_resets_selected_proxy_when_active_subscription_changes() {
        let mut settings = AppSettings::default();
        settings.proxy.selected_group = "Proxy".to_string();
        settings.proxy.selected_proxy = "Old Node".to_string();
        settings.proxy.active_subscription_id = "b".to_string();
        settings.proxy.subscriptions = vec![
            ProxySubscription {
                id: "a".to_string(),
                name: "A".to_string(),
                url: "https://example.com/a".to_string(),
            },
            ProxySubscription {
                id: "b".to_string(),
                name: "B".to_string(),
                url: "https://example.com/b".to_string(),
            },
        ];

        settings
            .proxy
            .normalize_subscriptions_with_previous_active(Some("a"));

        assert!(settings.proxy.selected_group.is_empty());
        assert!(settings.proxy.selected_proxy.is_empty());
    }
}
