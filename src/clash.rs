use std::{
    fs::{self, File},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Number, Value};

use crate::{
    AppSettings, ProxyScheme, ProxySettings, app_data_dir, controller::ClashController,
    stable_subscription_id,
};

const CLASH_DIR_NAME: &str = "clash";
const MIHOMO_EXE_NAME: &str = "mihomo.exe";
const SUBSCRIPTION_USER_AGENTS: &[&str] = &["clash.meta", "clash"];

#[cfg(windows)]
const CREATE_NO_WINDOW_FLAG: u32 = windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

pub struct ClashRuntime {
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<ChildJob>,
    mixed_port: u16,
    controller_port: u16,
    controller_secret: String,
    config_path: PathBuf,
}

#[cfg(windows)]
struct ChildJob(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
// Windows HANDLE values are process-wide kernel object references. This wrapper only closes the
// handle on drop and does not expose shared interior state, so moving it between threads is safe.
unsafe impl Send for ChildJob {}

#[cfg(windows)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePorts {
    pub mixed_port: u16,
    pub controller_port: u16,
}

impl ClashRuntime {
    pub fn proxy_settings(&self) -> ProxySettings {
        ProxySettings {
            scheme: ProxyScheme::Http,
            host: "127.0.0.1".to_string(),
            port: self.mixed_port.to_string(),
        }
    }

    pub fn controller(&self) -> Result<ClashController> {
        ClashController::new(self.controller_port, self.controller_secret.clone())
    }

    pub fn mixed_port(&self) -> u16 {
        self.mixed_port
    }

    pub fn controller_port(&self) -> u16 {
        self.controller_port
    }

    pub fn refresh_config(&self, settings: &AppSettings) -> Result<()> {
        let subscription_path = active_subscription_cache_path(settings)?;
        let subscription_url = settings.proxy.active_subscription_url();
        let subscription_text = if subscription_url.trim().is_empty() {
            read_cached_subscription(&subscription_path)?
        } else {
            download_subscription(subscription_url)?
        };
        let config_text = build_sanitized_config(
            &subscription_text,
            self.mixed_port,
            self.controller_port,
            &self.controller_secret,
        )?;

        write_subscription_cache(&subscription_path, &subscription_text)?;
        fs::write(&self.config_path, config_text).context("could not write mihomo config.yaml")?;
        self.controller()?.reload_config(&self.config_path)
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(windows)]
        {
            self.job.take();
        }
    }
}

impl Drop for ClashRuntime {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_internal_clash(settings: &AppSettings) -> Result<ClashRuntime> {
    let runtime_dir = clash_runtime_dir()?;
    let logs_dir = runtime_dir.join("logs");
    fs::create_dir_all(&logs_dir).with_context(|| {
        format!(
            "could not create Clash log directory at {}",
            logs_dir.display()
        )
    })?;

    let mihomo_path = bundled_mihomo_path()?;
    cleanup_stale_bundled_mihomo(&mihomo_path);
    let ports = resolved_runtime_ports(settings)?;
    let mixed_port = ports.mixed_port;
    let controller_port = ports.controller_port;
    let secret = generate_controller_secret();

    let subscription_path = active_subscription_cache_path(settings)?;
    ensure_subscription_source_available(
        settings.proxy.active_subscription_url(),
        &subscription_path,
        settings.proxy.auto_update_subscription,
    )?;
    let subscription_text = load_startup_subscription(
        settings.proxy.active_subscription_url(),
        &subscription_path,
        settings.proxy.auto_update_subscription,
    )?;
    let config_text =
        build_sanitized_config(&subscription_text, mixed_port, controller_port, &secret)?;

    write_subscription_cache(&subscription_path, &subscription_text)?;
    let config_path = runtime_dir.join("config.yaml");
    fs::write(&config_path, config_text).context("could not write mihomo config.yaml")?;

    let log = File::create(logs_dir.join("mihomo.log")).context("could not create mihomo log")?;
    let mut command = mihomo_command(&mihomo_path, &runtime_dir, &config_path, &log)?;
    let child = command
        .spawn()
        .with_context(|| format!("could not start {}", mihomo_path.display()))?;

    let mut child = child;
    #[cfg(windows)]
    let job = match attach_child_to_kill_on_close_job(&child) {
        Ok(job) => Some(job),
        Err(error) => {
            eprintln!("could not attach mihomo to cleanup job object: {error:#}");
            None
        }
    };
    if let Err(error) = wait_for_controller(controller_port, &secret) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }

    if let Some((group, proxy)) = saved_proxy_selection(settings) {
        let controller = ClashController::new(controller_port, secret.clone())?;
        if let Err(error) = controller.select_proxy(group, proxy) {
            eprintln!("could not restore saved proxy selection: {error:#}");
        }
    }

    Ok(ClashRuntime {
        child: Some(child),
        #[cfg(windows)]
        job,
        mixed_port,
        controller_port,
        controller_secret: secret,
        config_path,
    })
}

fn mihomo_command(
    mihomo_path: &Path,
    runtime_dir: &Path,
    config_path: &Path,
    log: &File,
) -> Result<Command> {
    let mut command = Command::new(mihomo_path);
    command
        .arg("-d")
        .arg(runtime_dir)
        .arg("-f")
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log.try_clone()?));

    hide_child_console_window(&mut command);

    Ok(command)
}

#[cfg(windows)]
fn attach_child_to_kill_on_close_job(child: &Child) -> Result<ChildJob> {
    use std::mem::size_of;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        anyhow::bail!(
            "could not create Windows cleanup job object: {}",
            std::io::Error::last_os_error()
        );
    }

    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const core::ffi::c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if configured == 0 {
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(job);
        }
        anyhow::bail!(
            "could not configure Windows cleanup job object: {}",
            std::io::Error::last_os_error()
        );
    }

    let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as _) };
    if assigned == 0 {
        unsafe {
            let _ = windows_sys::Win32::Foundation::CloseHandle(job);
        }
        anyhow::bail!(
            "could not assign mihomo to Windows cleanup job object: {}",
            std::io::Error::last_os_error()
        );
    }

    Ok(ChildJob(job))
}

fn clash_runtime_dir() -> Result<PathBuf> {
    Ok(app_data_dir()?.join(CLASH_DIR_NAME))
}

fn active_subscription_cache_path(settings: &AppSettings) -> Result<PathBuf> {
    let runtime_dir = clash_runtime_dir()?;
    let cache_id = active_subscription_cache_id(settings);
    if cache_id.is_empty() {
        return Ok(runtime_dir.join("subscription.yaml"));
    }

    let cache_path = runtime_dir
        .join("subscriptions")
        .join(subscription_cache_file_name(&cache_id));
    migrate_single_subscription_cache_if_safe(settings, &cache_path)?;

    Ok(cache_path)
}

fn active_subscription_cache_id(settings: &AppSettings) -> String {
    let active_id = settings.proxy.active_subscription_id.trim();
    if !active_id.is_empty() {
        return active_id.to_string();
    }

    let active_url = settings.proxy.active_subscription_url().trim();
    if active_url.is_empty() {
        String::new()
    } else {
        stable_subscription_id(active_url)
    }
}

fn subscription_cache_file_name(cache_id: &str) -> String {
    let sanitized = cache_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    let sanitized = sanitized.trim_matches('_');
    let sanitized = if sanitized.is_empty() {
        "subscription"
    } else {
        sanitized
    };

    format!("{sanitized}.yaml")
}

fn migrate_single_subscription_cache_if_safe(
    settings: &AppSettings,
    cache_path: &Path,
) -> Result<()> {
    if cache_path.exists() || settings.proxy.subscriptions.len() > 1 {
        return Ok(());
    }

    let legacy_path = clash_runtime_dir()?.join("subscription.yaml");
    if !legacy_path.exists() {
        return Ok(());
    }

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create subscription cache directory at {}",
                parent.display()
            )
        })?;
    }
    fs::copy(&legacy_path, cache_path).with_context(|| {
        format!(
            "could not migrate subscription cache from {} to {}",
            legacy_path.display(),
            cache_path.display()
        )
    })?;

    Ok(())
}

fn write_subscription_cache(subscription_path: &Path, subscription_text: &str) -> Result<()> {
    if let Some(parent) = subscription_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create subscription cache directory at {}",
                parent.display()
            )
        })?;
    }

    fs::write(subscription_path, subscription_text)
        .with_context(|| format!("could not write {}", subscription_path.display()))
}

pub fn read_mihomo_log_tail(max_lines: usize) -> Result<String> {
    let log_path = clash_runtime_dir()?.join("logs").join("mihomo.log");
    let contents = fs::read_to_string(&log_path)
        .with_context(|| format!("could not read mihomo log at {}", log_path.display()))?;
    let lines = contents.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(max_lines);

    Ok(lines[start..].join("\n"))
}

fn bundled_mihomo_path() -> Result<PathBuf> {
    let current_exe = std::env::current_exe().context("could not locate current executable")?;
    let exe_dir = current_exe
        .parent()
        .context("could not locate current executable directory")?;

    let release_sidecar = exe_dir
        .join("resources")
        .join(CLASH_DIR_NAME)
        .join(MIHOMO_EXE_NAME);
    if release_sidecar.exists() {
        return Ok(release_sidecar);
    }

    let dev_sidecar = std::env::current_dir()
        .context("could not locate current working directory")?
        .join("resources")
        .join(CLASH_DIR_NAME)
        .join(MIHOMO_EXE_NAME);
    if dev_sidecar.exists() {
        return Ok(dev_sidecar);
    }

    bail!(
        "could not find bundled mihomo.exe. Expected {} or {}",
        release_sidecar.display(),
        dev_sidecar.display()
    )
}

fn configured_runtime_ports(settings: &AppSettings) -> Result<(u16, u16)> {
    let mixed_port = settings.proxy.mixed_port;
    let controller_port = settings.proxy.controller_port;

    if mixed_port == controller_port {
        bail!("internal Clash proxy port and controller port must be different");
    }

    Ok((mixed_port, controller_port))
}

pub fn resolved_runtime_ports(settings: &AppSettings) -> Result<RuntimePorts> {
    let (mixed_port, controller_port) = configured_runtime_ports(settings)?;
    let mixed_port = resolve_available_port(mixed_port)?;
    let controller_port = if controller_port == mixed_port {
        find_available_loopback_port()?
    } else {
        resolve_available_port(controller_port)?
    };

    if mixed_port == controller_port {
        bail!("could not resolve different ports for internal Clash runtime");
    }

    Ok(RuntimePorts {
        mixed_port,
        controller_port,
    })
}

fn resolve_available_port(preferred_port: u16) -> Result<u16> {
    if port_is_available(preferred_port) {
        return Ok(preferred_port);
    }

    find_available_loopback_port()
}

fn port_is_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn find_available_loopback_port() -> Result<u16> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("could not find an available local port")?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .context("could not read available local port")
}

fn cleanup_stale_bundled_mihomo(mihomo_path: &Path) {
    if let Err(error) = cleanup_stale_bundled_mihomo_impl(mihomo_path) {
        eprintln!("could not clean stale bundled mihomo process: {error:#}");
    }
}

#[cfg(windows)]
fn cleanup_stale_bundled_mihomo_impl(mihomo_path: &Path) -> Result<()> {
    let target = mihomo_path
        .canonicalize()
        .unwrap_or_else(|_| mihomo_path.to_path_buf());
    let target = target.to_string_lossy();
    let escaped_target = target.replace('\'', "''");
    let command = format!(
        "$target = '{}'; Get-CimInstance Win32_Process -Filter \"Name = 'mihomo.exe'\" | Where-Object {{ $_.ExecutablePath -eq $target }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
        escaped_target
    );

    let mut cleanup_command = powershell_cleanup_command(command);
    let _ = cleanup_command
        .status()
        .context("could not invoke PowerShell to clean stale mihomo")?;

    Ok(())
}

#[cfg(not(windows))]
fn cleanup_stale_bundled_mihomo_impl(_mihomo_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn powershell_cleanup_command(command: String) -> Command {
    let mut powershell = Command::new("powershell");
    powershell
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    hide_child_console_window(&mut powershell);

    powershell
}

fn hide_child_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW_FLAG);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn download_subscription(url: &str) -> Result<String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(45))
        .build()
        .context("could not build subscription download client")?;
    let mut fallback = None;

    for user_agent in SUBSCRIPTION_USER_AGENTS {
        let subscription = download_subscription_with_user_agent(&client, url, user_agent)?;
        if subscription_is_valid_clash_config(&subscription) {
            return Ok(subscription);
        }
        fallback.get_or_insert(subscription);
    }

    fallback.context("could not download proxy subscription")
}

fn download_subscription_with_user_agent(
    client: &Client,
    url: &str,
    user_agent: &str,
) -> Result<String> {
    let response = client
        .get(url)
        .header("User-Agent", user_agent)
        .send()
        .with_context(|| {
            format!("could not download proxy subscription with User-Agent {user_agent}")
        })?
        .error_for_status()
        .with_context(|| {
            format!("proxy subscription returned an error status for User-Agent {user_agent}")
        })?;

    response.text().with_context(|| {
        format!("could not read proxy subscription response for User-Agent {user_agent}")
    })
}

#[cfg(test)]
fn load_or_download_subscription(
    url: &str,
    subscription_path: &Path,
    auto_update: bool,
) -> Result<String> {
    load_or_download_subscription_with(url, subscription_path, auto_update, download_subscription)
}

fn load_startup_subscription(
    url: &str,
    subscription_path: &Path,
    auto_update: bool,
) -> Result<String> {
    load_startup_subscription_with(url, subscription_path, auto_update, download_subscription)
}

fn load_startup_subscription_with<F>(
    url: &str,
    subscription_path: &Path,
    auto_update: bool,
    mut download: F,
) -> Result<String>
where
    F: FnMut(&str) -> Result<String>,
{
    ensure_subscription_source_available(url, subscription_path, auto_update)?;

    if subscription_path.exists() {
        let cached_subscription = read_cached_subscription(subscription_path)?;
        if !auto_update || url.trim().is_empty() {
            return Ok(cached_subscription);
        }

        return match download(url) {
            Ok(subscription) => Ok(subscription),
            Err(_error) if subscription_is_valid_clash_config(&cached_subscription) => {
                Ok(cached_subscription)
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "cached subscription at {} is not valid Clash YAML and could not be refreshed",
                    subscription_path.display()
                )
            }),
        };
    }

    load_or_download_subscription_with(url, subscription_path, auto_update, download)
}

fn load_or_download_subscription_with<F>(
    url: &str,
    subscription_path: &Path,
    auto_update: bool,
    mut download: F,
) -> Result<String>
where
    F: FnMut(&str) -> Result<String>,
{
    ensure_subscription_source_available(url, subscription_path, auto_update)?;

    if !auto_update && subscription_path.exists() {
        return read_cached_subscription(subscription_path);
    }

    if url.trim().is_empty() {
        return read_cached_subscription(subscription_path);
    }

    match download(url) {
        Ok(subscription) => Ok(subscription),
        Err(error) if subscription_path.exists() => read_cached_subscription(subscription_path)
            .with_context(|| {
                format!("could not download subscription ({error:#}) or use cached subscription")
            }),
        Err(error) => Err(error),
    }
}

fn ensure_subscription_source_available(
    url: &str,
    subscription_path: &Path,
    _auto_update: bool,
) -> Result<()> {
    if subscription_path.exists() {
        return Ok(());
    }

    if url.trim().is_empty() {
        bail!(
            "internal Clash mode requires a subscription URL, or an existing cached subscription with auto-update disabled"
        );
    }

    Ok(())
}

fn read_cached_subscription(subscription_path: &Path) -> Result<String> {
    fs::read_to_string(subscription_path)
        .with_context(|| format!("could not read {}", subscription_path.display()))
}

fn build_sanitized_config(
    subscription_text: &str,
    mixed_port: u16,
    controller_port: u16,
    secret: &str,
) -> Result<String> {
    let subscription_text = normalize_subscription_text(subscription_text);
    let source: Value =
        serde_yaml::from_str(subscription_text).context("subscription is not valid YAML")?;
    let Some(source) = source.as_mapping() else {
        anyhow::bail!(
            "subscription response is not Clash YAML; provider may require Clash User-Agent or a Clash-format subscription URL"
        );
    };

    let mut target = Mapping::new();
    copy_yaml_key(source, &mut target, "proxies");
    copy_yaml_key(source, &mut target, "proxy-providers");
    copy_yaml_key(source, &mut target, "proxy-groups");
    validate_required_subscription_keys(&target)?;
    let app_group_name = first_proxy_group_name(&target)?;

    target.insert(str_value("mode"), str_value("rule"));
    target.insert(str_value("log-level"), str_value("warning"));
    target.insert(str_value("allow-lan"), Value::Bool(false));
    target.insert(str_value("bind-address"), str_value("127.0.0.1"));
    target.insert(
        str_value("mixed-port"),
        Value::Number(Number::from(mixed_port)),
    );
    target.insert(
        str_value("external-controller"),
        str_value(&format!("127.0.0.1:{controller_port}")),
    );
    target.insert(str_value("secret"), str_value(secret));
    target.insert(
        str_value("rules"),
        Value::Sequence(vec![str_value(&format!("MATCH,{app_group_name}"))]),
    );

    serde_yaml::to_string(&Value::Mapping(target)).context("could not render mihomo config")
}

fn validate_required_subscription_keys(config: &Mapping) -> Result<()> {
    let mut missing = Vec::new();

    let key = "proxy-groups";
    let key_value = str_value(key);
    let has_non_empty_sequence = config
        .get(&key_value)
        .and_then(Value::as_sequence)
        .is_some_and(|items| !items.is_empty());

    if !has_non_empty_sequence {
        missing.push(key);
    }

    let has_inline_proxies = config
        .get(str_value("proxies"))
        .and_then(Value::as_sequence)
        .is_some_and(|items| !items.is_empty());
    let has_proxy_providers = config
        .get(str_value("proxy-providers"))
        .and_then(Value::as_mapping)
        .is_some_and(|providers| !providers.is_empty());

    if !has_inline_proxies && !has_proxy_providers {
        missing.push("proxies or proxy-providers");
    }

    if !missing.is_empty() {
        bail!(
            "subscription is missing required Clash routing keys: {}",
            missing.join(", ")
        );
    }

    Ok(())
}

fn first_proxy_group_name(config: &Mapping) -> Result<String> {
    let groups = config
        .get(str_value("proxy-groups"))
        .and_then(Value::as_sequence)
        .context("subscription is missing proxy-groups")?;
    let group = groups
        .first()
        .and_then(Value::as_mapping)
        .and_then(|group| group.get(str_value("name")))
        .and_then(Value::as_str)
        .context("first proxy group is missing a name")?;

    Ok(group.to_string())
}

fn copy_yaml_key(source: &Mapping, target: &mut Mapping, key: &str) {
    let key_value = str_value(key);
    if let Some(value) = source.get(&key_value) {
        target.insert(key_value, value.clone());
    }
}

fn normalize_subscription_text(subscription_text: &str) -> &str {
    subscription_text.trim_start_matches('\u{feff}')
}

fn subscription_is_valid_clash_config(subscription_text: &str) -> bool {
    build_sanitized_config(subscription_text, 17898, 17899, "startup-cache-check").is_ok()
}

fn str_value(value: &str) -> Value {
    Value::String(value.to_string())
}

fn generate_controller_secret() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}-{}", std::process::id())
}

fn wait_for_controller(port: u16, secret: &str) -> Result<()> {
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let url = format!("http://127.0.0.1:{port}/configs");

    for _ in 0..30 {
        let response = client
            .get(&url)
            .bearer_auth(secret)
            .send()
            .and_then(|response| response.error_for_status());

        if response.is_ok() {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(500));
    }

    bail!("mihomo did not become ready at {url}")
}

fn saved_proxy_selection(settings: &AppSettings) -> Option<(&str, &str)> {
    let group = settings.proxy.selected_group.trim();
    let proxy = settings.proxy.selected_proxy.trim();

    if group.is_empty() || proxy.is_empty() {
        return None;
    }

    Some((group, proxy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    fn sample_subscription() -> &'static str {
        r#"
mixed-port: 7890
allow-lan: true
proxies:
  - name: test
    type: direct
proxy-groups:
  - name: AUTO
    type: select
    proxies:
      - test
rules:
  - MATCH,AUTO
"#
    }

    fn alternate_subscription() -> &'static str {
        r#"
proxies:
  - name: fresh
    type: direct
proxy-groups:
  - name: FreshProxy
    type: select
    proxies:
      - fresh
rules:
  - MATCH,FreshProxy
"#
    }

    fn multi_node_subscription() -> &'static str {
        r#"
mixed-port: 7890
proxies:
  - { name: hk-1, type: ss, server: example.com, port: 10001, cipher: aes-128-gcm, password: test }
  - { name: jp-1, type: ss, server: example.com, port: 10002, cipher: aes-128-gcm, password: test }
  - { name: us-1, type: ss, server: example.com, port: 10003, cipher: aes-128-gcm, password: test }
proxy-groups:
  - { name: Proxy, type: select, proxies: [hk-1, jp-1, us-1] }
rules:
  - MATCH,Proxy
"#
    }

    #[test]
    fn download_subscription_prefers_clash_meta_user_agent_for_complete_node_lists() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let url = format!(
            "http://127.0.0.1:{}/subscribe",
            listener.local_addr().expect("server address").port()
        );
        let compact_yaml = sample_subscription().trim_start().to_string();
        let full_yaml = multi_node_subscription().trim_start().to_string();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test request should arrive");
            let user_agent = read_test_user_agent(&mut stream);
            let body = if user_agent == "clash.meta" {
                full_yaml
            } else {
                compact_yaml
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );

            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let downloaded = download_subscription(&url).expect("subscription should download");
        handle.join().expect("test server should finish");

        assert!(
            downloaded.contains("jp-1"),
            "provider returned compact content: {downloaded}"
        );
    }

    #[test]
    fn download_subscription_falls_back_to_clash_user_agent_when_meta_is_not_clash_yaml() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let url = format!(
            "http://127.0.0.1:{}/subscribe",
            listener.local_addr().expect("server address").port()
        );
        let expected_yaml = sample_subscription().trim_start().to_string();

        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("test request should arrive");
                let user_agent = read_test_user_agent(&mut stream);
                let body = if user_agent == "clash" {
                    expected_yaml.clone()
                } else {
                    "dm1lc3M6Ly90ZXN0".to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );

                stream
                    .write_all(response.as_bytes())
                    .expect("response should be written");
            }
        });

        let downloaded = download_subscription(&url).expect("subscription should download");
        handle.join().expect("test server should finish");

        assert!(
            downloaded.contains("proxy-groups:"),
            "provider returned non-Clash content: {downloaded}"
        );
    }

    fn read_test_user_agent(stream: &mut std::net::TcpStream) -> String {
        let mut request = [0_u8; 4096];
        let bytes = stream.read(&mut request).expect("request should be read");
        let request = String::from_utf8_lossy(&request[..bytes]);
        request
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
                    .map(|(_, value)| value.trim().to_string())
            })
            .unwrap_or_default()
    }

    #[test]
    fn sanitized_config_rejects_plain_node_subscription_with_clear_message() {
        let error = build_sanitized_config("dm1lc3M6Ly90ZXN0", 17898, 17899, "secret")
            .expect_err("plain node subscriptions should be rejected");

        assert!(
            error.to_string().contains("not Clash YAML"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn sanitized_config_forces_local_only_ports() {
        let config = build_sanitized_config(sample_subscription(), 17898, 17899, "secret")
            .expect("config should render");

        assert!(config.contains("mixed-port: 17898"));
        assert!(config.contains("allow-lan: false"));
        assert!(config.contains("bind-address: 127.0.0.1"));
        assert!(config.contains("external-controller: 127.0.0.1:17899"));
        assert!(config.contains("secret: secret"));
    }

    #[test]
    fn sanitized_config_rejects_subscription_without_required_routing_keys() {
        let subscription = r#"
proxies:
  - name: test
    type: direct
"#;

        let error = build_sanitized_config(subscription, 17898, 17899, "secret")
            .expect_err("config without proxy-groups and rules should be rejected");

        assert!(error.to_string().contains("proxy-groups"));
    }

    #[test]
    fn sanitized_config_accepts_utf8_bom_subscription_cache() {
        let subscription = format!("\u{feff}{}", sample_subscription());

        let config = build_sanitized_config(&subscription, 17898, 17899, "secret")
            .expect("BOM-prefixed subscription should render");

        assert!(config.contains("mixed-port: 17898"));
    }

    #[test]
    fn sanitized_config_accepts_bom_without_leading_newline() {
        let subscription = "\u{feff}proxies:\n  - name: direct-test\n    type: direct\nproxy-groups:\n  - name: AUTO\n    type: select\n    proxies:\n      - direct-test\n      - DIRECT\nrules:\n  - MATCH,AUTO\n";

        let config = build_sanitized_config(subscription, 17898, 17899, "secret")
            .expect("BOM-prefixed subscription should render");

        assert!(config.contains("mixed-port: 17898"));
    }

    #[test]
    fn sanitized_config_rewrites_rules_to_app_proxy_group() {
        let subscription = r#"
proxies:
  - name: test
    type: direct
proxy-groups:
  - name: Proxy
    type: select
    proxies:
      - test
rules:
  - GEOIP,CN,DIRECT
  - MATCH,Proxy
"#;

        let config = build_sanitized_config(subscription, 17898, 17899, "secret")
            .expect("config should render");

        assert!(config.contains("- MATCH,Proxy"));
        assert!(!config.contains("GEOIP"));
    }

    #[test]
    fn cached_subscription_can_start_without_subscription_url_when_auto_update_is_disabled() {
        let cache_dir = std::env::temp_dir().join(format!(
            "chatgpt-client-clash-test-{}",
            generate_controller_secret()
        ));
        fs::create_dir_all(&cache_dir).expect("cache dir should be created");
        let subscription_path = cache_dir.join("subscription.yaml");
        fs::write(&subscription_path, sample_subscription()).expect("cache should be written");

        let result = load_or_download_subscription("", &subscription_path, false);

        let _ = fs::remove_dir_all(&cache_dir);
        assert_eq!(
            result.expect("cached subscription should load"),
            sample_subscription()
        );
    }

    #[test]
    fn empty_subscription_url_requires_cache_or_auto_update_disabled() {
        let subscription_path = std::env::temp_dir().join(format!(
            "missing-chatgpt-client-subscription-{}.yaml",
            generate_controller_secret()
        ));

        let error = ensure_subscription_source_available("", &subscription_path, false)
            .expect_err("missing cache and empty URL should fail");

        assert!(error.to_string().contains("subscription URL"));
    }

    #[test]
    fn cached_subscription_is_used_when_auto_update_download_fails() {
        let cache_dir = std::env::temp_dir().join(format!(
            "chatgpt-client-clash-fallback-test-{}",
            generate_controller_secret()
        ));
        fs::create_dir_all(&cache_dir).expect("cache dir should be created");
        let subscription_path = cache_dir.join("subscription.yaml");
        fs::write(&subscription_path, sample_subscription()).expect("cache should be written");

        let result = load_or_download_subscription_with(
            "https://example.invalid/sub",
            &subscription_path,
            true,
            |_| bail!("network unavailable"),
        );

        let _ = fs::remove_dir_all(&cache_dir);
        assert_eq!(
            result.expect("cached subscription should be used as fallback"),
            sample_subscription()
        );
    }

    #[test]
    fn startup_auto_update_downloads_active_subscription_even_when_cache_exists() {
        let cache_dir = std::env::temp_dir().join(format!(
            "chatgpt-client-clash-refresh-active-test-{}",
            generate_controller_secret()
        ));
        fs::create_dir_all(&cache_dir).expect("cache dir should be created");
        let subscription_path = cache_dir.join("subscription.yaml");
        fs::write(&subscription_path, sample_subscription()).expect("cache should be written");

        let result = load_startup_subscription_with(
            "https://example.invalid/sub",
            &subscription_path,
            true,
            |_| Ok(alternate_subscription().to_string()),
        );

        let _ = fs::remove_dir_all(&cache_dir);
        assert_eq!(
            result.expect("startup should download the active subscription"),
            alternate_subscription()
        );
    }

    #[test]
    fn startup_uses_valid_cache_when_auto_update_download_fails() {
        let cache_dir = std::env::temp_dir().join(format!(
            "chatgpt-client-clash-startup-fallback-test-{}",
            generate_controller_secret()
        ));
        fs::create_dir_all(&cache_dir).expect("cache dir should be created");
        let subscription_path = cache_dir.join("subscription.yaml");
        fs::write(&subscription_path, sample_subscription()).expect("cache should be written");

        let result = load_startup_subscription_with(
            "https://example.invalid/sub",
            &subscription_path,
            true,
            |_| bail!("network unavailable"),
        );

        let _ = fs::remove_dir_all(&cache_dir);
        assert_eq!(
            result.expect("valid cache should be used when refresh fails"),
            sample_subscription()
        );
    }

    #[test]
    fn invalid_cached_subscription_is_refreshed_on_startup_when_auto_update_is_enabled() {
        let cache_dir = std::env::temp_dir().join(format!(
            "chatgpt-client-clash-invalid-cache-test-{}",
            generate_controller_secret()
        ));
        fs::create_dir_all(&cache_dir).expect("cache dir should be created");
        let subscription_path = cache_dir.join("subscription.yaml");
        fs::write(&subscription_path, "dm1lc3M6Ly90ZXN0").expect("cache should be written");

        let result = load_startup_subscription_with(
            "https://example.test/sub",
            &subscription_path,
            true,
            |_| Ok(sample_subscription().to_string()),
        );

        let _ = fs::remove_dir_all(&cache_dir);
        assert_eq!(
            result.expect("startup should refresh invalid cached subscription"),
            sample_subscription()
        );
    }

    #[test]
    fn saved_proxy_selection_requires_group_and_proxy() {
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port: 17898,
                controller_port: 17899,
                auto_update_subscription: true,
                selected_group: "PROXY".to_string(),
                selected_proxy: String::new(),
            },
        };

        assert!(saved_proxy_selection(&settings).is_none());
    }

    #[test]
    fn saved_proxy_selection_returns_group_and_proxy() {
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port: 17898,
                controller_port: 17899,
                auto_update_subscription: true,
                selected_group: "PROXY".to_string(),
                selected_proxy: "香港 01".to_string(),
            },
        };

        assert_eq!(saved_proxy_selection(&settings), Some(("PROXY", "香港 01")));
    }

    #[test]
    fn runtime_ports_use_configured_settings() {
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port: 19080,
                controller_port: 19081,
                auto_update_subscription: true,
                selected_group: String::new(),
                selected_proxy: String::new(),
            },
        };

        let (mixed_port, controller_port) =
            configured_runtime_ports(&settings).expect("ports should be accepted");

        assert_eq!(mixed_port, 19080);
        assert_eq!(controller_port, 19081);
    }

    #[test]
    fn runtime_ports_reject_equal_ports() {
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port: 19080,
                controller_port: 19080,
                auto_update_subscription: true,
                selected_group: String::new(),
                selected_proxy: String::new(),
            },
        };

        let error = configured_runtime_ports(&settings).expect_err("same port should be rejected");

        assert!(error.to_string().contains("must be different"));
    }

    #[test]
    fn resolved_runtime_ports_preserve_free_configured_ports() {
        let mixed_listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("mixed test port should bind");
        let controller_listener =
            TcpListener::bind(("127.0.0.1", 0)).expect("controller test port should bind");
        let mixed_port = mixed_listener.local_addr().expect("mixed addr").port();
        let controller_port = controller_listener
            .local_addr()
            .expect("controller addr")
            .port();
        drop(mixed_listener);
        drop(controller_listener);
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port,
                controller_port,
                auto_update_subscription: true,
                selected_group: String::new(),
                selected_proxy: String::new(),
            },
        };

        let ports = resolved_runtime_ports(&settings).expect("ports should resolve");

        assert_eq!(ports.mixed_port, mixed_port);
        assert_eq!(ports.controller_port, controller_port);
    }

    #[test]
    fn resolved_runtime_ports_avoid_occupied_ports() {
        let occupied_mixed =
            TcpListener::bind(("127.0.0.1", 0)).expect("mixed occupied port should bind");
        let occupied_controller =
            TcpListener::bind(("127.0.0.1", 0)).expect("controller occupied port should bind");
        let mixed_port = occupied_mixed.local_addr().expect("mixed addr").port();
        let controller_port = occupied_controller
            .local_addr()
            .expect("controller addr")
            .port();
        let settings = AppSettings {
            proxy: crate::ProxyUiSettings {
                mode: crate::ProxyMode::InternalClash,
                subscription_url: String::new(),
                active_subscription_id: String::new(),
                subscriptions: Vec::new(),
                mixed_port,
                controller_port,
                auto_update_subscription: true,
                selected_group: String::new(),
                selected_proxy: String::new(),
            },
        };

        let ports = resolved_runtime_ports(&settings).expect("ports should resolve");

        assert_ne!(ports.mixed_port, mixed_port);
        assert_ne!(ports.controller_port, controller_port);
        assert_ne!(ports.mixed_port, ports.controller_port);
    }

    #[test]
    fn active_subscription_cache_is_isolated_by_subscription_id() {
        let mut settings = AppSettings::default();
        settings.proxy.subscriptions = vec![
            crate::ProxySubscription {
                id: "sub-a".to_string(),
                name: "A".to_string(),
                url: "https://example.com/a".to_string(),
            },
            crate::ProxySubscription {
                id: "sub-b".to_string(),
                name: "B".to_string(),
                url: "https://example.com/b".to_string(),
            },
        ];
        settings.proxy.active_subscription_id = "sub-a".to_string();
        settings.proxy.normalize_subscriptions();

        let first = active_subscription_cache_path(&settings).expect("first cache path");
        settings.proxy.active_subscription_id = "sub-b".to_string();
        settings.proxy.normalize_subscriptions();
        let second = active_subscription_cache_path(&settings).expect("second cache path");

        assert_ne!(first, second);
        assert!(
            first
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("subscriptions/sub-a.yaml")
        );
        assert!(
            second
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with("subscriptions/sub-b.yaml")
        );
    }

    #[test]
    fn subscription_cache_id_sanitizes_path_separators() {
        assert_eq!(
            subscription_cache_file_name("sub/../bad"),
            "sub____bad.yaml"
        );
    }
}
