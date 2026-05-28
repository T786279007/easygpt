#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use chatgpt_webview_client::{
    AppSettings, CHATGPT_URL, ProxyMode, ProxyScheme, ProxySettings,
    clash::{ClashRuntime, read_mihomo_log_tail, resolved_runtime_ports, start_internal_clash},
    controller::{ProxyGroup, ProxyState, preferred_proxy_group, preferred_proxy_node},
    ensure_webview_profile_dir, load_settings, save_settings, startup_proxy,
};
use serde_json::{Value, json};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
    window::{Window, WindowBuilder},
};
use urlencoding::encode;
use wry::{
    NewWindowResponse, ProxyConfig, ProxyEndpoint, WebContext, WebView, WebViewBuilder,
    http::{Response, header::CONTENT_TYPE},
};

#[cfg(windows)]
use wry::{MemoryUsageLevel, WebViewBuilderExtWindows, WebViewExtWindows};

#[cfg(windows)]
const APP_MUTEX_NAME: &str = "Local\\ChatGPTWebviewClientSingleInstance";
const DELAY_TEST_URL: &str = "https://chatgpt.com/cdn-cgi/trace";
const DELAY_TIMEOUT_MS: u64 = 5000;
const TOP_BAR_HEIGHT: f64 = 52.0;
const SHELL_PROTOCOL: &str = "aiclient";
const SHELL_URL: &str = "aiclient://shell/index.html";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AiSite {
    ChatGpt,
    Gemini,
    NotebookLm,
    AiStudio,
}

impl AiSite {
    const ALL: [AiSite; 4] = [
        AiSite::ChatGpt,
        AiSite::Gemini,
        AiSite::NotebookLm,
        AiSite::AiStudio,
    ];

    fn all() -> &'static [AiSite] {
        &Self::ALL
    }

    fn key(self) -> &'static str {
        match self {
            AiSite::ChatGpt => "chatgpt",
            AiSite::Gemini => "gemini",
            AiSite::NotebookLm => "notebooklm",
            AiSite::AiStudio => "aistudio",
        }
    }

    fn title(self) -> &'static str {
        match self {
            AiSite::ChatGpt => "ChatGPT",
            AiSite::Gemini => "Gemini",
            AiSite::NotebookLm => "NotebookLM",
            AiSite::AiStudio => "Google AI Studio",
        }
    }

    fn url(self) -> &'static str {
        match self {
            AiSite::ChatGpt => CHATGPT_URL,
            AiSite::Gemini => "https://gemini.google.com",
            AiSite::NotebookLm => "https://notebooklm.google.com",
            AiSite::AiStudio => "https://aistudio.google.com",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::all().iter().copied().find(|site| site.key() == key)
    }
}

enum UserEvent {
    Ipc { target: IpcTarget, body: String },
    IpcResponse { target: IpcTarget, response: String },
    RuntimeReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum IpcTarget {
    Shell,
    Site(AiSite),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellCommand {
    SwitchSite(AiSite),
    CloseSite(AiSite),
    OptimizeMemory,
    NavBack,
    NavForward,
    ReloadActive,
}

struct AppRuntimeState {
    settings: AppSettings,
    clash_runtime: Option<ClashRuntime>,
    runtime_error: Option<String>,
    last_health: RuntimeHealth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeHealth {
    Unknown,
    Running,
    Degraded,
    Stopped,
}

impl AppRuntimeState {
    fn restart_clash_runtime(&mut self) -> bool {
        if let Some(runtime) = self.clash_runtime.as_mut() {
            runtime.stop();
        }
        self.clash_runtime = None;
        self.runtime_error = None;

        if matches!(self.settings.proxy.mode, ProxyMode::InternalClash) {
            match start_internal_clash(&self.settings) {
                Ok(runtime) => {
                    self.clash_runtime = Some(runtime);
                    if let Err(error) = self.ensure_valid_proxy_selection() {
                        eprintln!("could not ensure valid proxy selection: {error:#}");
                    }
                    self.last_health = RuntimeHealth::Running;
                    true
                }
                Err(error) => {
                    self.runtime_error = Some(format!("{error:#}"));
                    self.last_health = RuntimeHealth::Degraded;
                    false
                }
            }
        } else {
            self.last_health = RuntimeHealth::Stopped;
            true
        }
    }

    fn ensure_clash_runtime(&mut self) -> Result<()> {
        if self.clash_runtime.is_none()
            && matches!(self.settings.proxy.mode, ProxyMode::InternalClash)
            && !self.restart_clash_runtime()
        {
            anyhow::bail!(
                "{}",
                self.runtime_error
                    .as_deref()
                    .unwrap_or("internal Clash runtime is not running")
            );
        }

        Ok(())
    }

    fn health_label(&self) -> &'static str {
        match self.last_health {
            RuntimeHealth::Unknown => "unknown",
            RuntimeHealth::Running => "running",
            RuntimeHealth::Degraded => "degraded",
            RuntimeHealth::Stopped => "stopped",
        }
    }

    fn ensure_valid_proxy_selection(&mut self) -> Result<Option<(String, String)>> {
        let Some(runtime) = self.clash_runtime.as_ref() else {
            return Ok(None);
        };
        let controller = runtime.controller()?;
        let proxy_state = controller.proxy_state()?;
        let Some(group) = preferred_proxy_group(&proxy_state, &self.settings.proxy.selected_group)
        else {
            return Ok(None);
        };
        let Some(node) = preferred_proxy_node(group, &self.settings.proxy.selected_proxy) else {
            return Ok(None);
        };

        let group_name = group.name.clone();
        let node_name = node.name.clone();
        if group.selected != node_name {
            controller.select_proxy(&group_name, &node_name)?;
        }

        if self.settings.proxy.selected_group != group_name
            || self.settings.proxy.selected_proxy != node_name
        {
            self.settings.proxy.selected_group = group_name.clone();
            self.settings.proxy.selected_proxy = node_name.clone();
            save_settings(&self.settings)?;
        }

        Ok(Some((group_name, node_name)))
    }

    fn repair_proxy(&mut self) -> Result<ProxyRepairResult> {
        if !self.restart_clash_runtime() {
            anyhow::bail!(
                "{}",
                self.runtime_error
                    .as_deref()
                    .unwrap_or("internal Clash runtime could not be started")
            );
        }

        let runtime = self
            .clash_runtime
            .as_ref()
            .context("internal Clash runtime is not running")?;
        let controller = runtime.controller()?;
        let proxy_state = controller.proxy_state()?;
        let candidates = ordered_proxy_candidates(
            &proxy_state.groups,
            &self.settings.proxy.selected_group,
            &self.settings.proxy.selected_proxy,
        );

        let mut last_error = None;
        for (group, proxy) in candidates {
            if let Err(error) = controller.select_proxy(&group, &proxy) {
                last_error = Some(error.to_string());
                continue;
            }

            let delay = controller.test_delay(&group, DELAY_TEST_URL, DELAY_TIMEOUT_MS);
            if delay.error.is_none() {
                self.settings.proxy.selected_group = group.clone();
                self.settings.proxy.selected_proxy = proxy.clone();
                save_settings(&self.settings)?;

                return Ok(ProxyRepairResult {
                    group,
                    proxy,
                    delay_ms: delay.delay_ms,
                    checked: true,
                });
            }

            last_error = delay.error;
        }

        anyhow::bail!(
            "no usable proxy node reached ChatGPT{}",
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    }
}

#[derive(Debug, Clone, serde::Serialize)]
struct ProxyRepairResult {
    group: String,
    proxy: String,
    delay_ms: Option<u64>,
    checked: bool,
}

fn main() {
    if let Err(error) = run_app() {
        let message = format!("{error:#}");
        eprintln!("{message}");
        show_startup_error(&message);
        std::process::exit(1);
    }
}

fn run_app() -> Result<()> {
    let _single_instance = acquire_single_instance()?;
    let profile_dir = ensure_webview_profile_dir()?;
    let mut settings = load_settings();
    heal_startup_runtime_ports(&mut settings)?;
    save_settings(&settings)?;
    let settings_json =
        serde_json::to_string(&settings).context("could not serialize settings for UI")?;
    let mut initial_state = AppRuntimeState {
        settings: settings.clone(),
        clash_runtime: None,
        runtime_error: None,
        last_health: RuntimeHealth::Unknown,
    };
    if !matches!(settings.proxy.mode, ProxyMode::InternalClash) {
        initial_state.last_health = RuntimeHealth::Stopped;
    }
    let app_state = Arc::new(Mutex::new(initial_state));
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let event_proxy = event_loop.create_proxy();
    spawn_runtime_watchdog(Arc::clone(&app_state));
    if matches!(settings.proxy.mode, ProxyMode::InternalClash) {
        spawn_initial_runtime_start(Arc::clone(&app_state), event_proxy.clone());
    }
    let window = WindowBuilder::new()
        .with_title("AI Web Client")
        .with_inner_size(LogicalSize::new(1280.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0))
        .build(&event_loop)
        .context("could not create the main window")?;

    let mut web_context = WebContext::new(Some(profile_dir));
    let detected_proxy = startup_proxy(&settings);
    let initial_site = AiSite::ChatGpt;
    let mut active_site = initial_site;
    let mut runtime_ready = !matches!(settings.proxy.mode, ProxyMode::InternalClash);
    let (window_width, window_height) = logical_window_size(&window);
    let shell_webview = build_shell_webview(
        &window,
        &mut web_context,
        initial_site,
        top_bar_bounds(window_width),
        detected_proxy.as_ref(),
        event_proxy.clone(),
    )
    .context("could not create the top navigation WebView2 instance")?;
    let mut content_webviews = HashMap::new();
    let initial_content = build_content_webview(
        &window,
        &mut web_context,
        initial_site,
        content_bounds(window_width, window_height),
        &settings,
        &settings_json,
        detected_proxy.as_ref(),
        runtime_ready,
        true,
        event_proxy.clone(),
    )
    .context("could not create the initial content WebView2 instance")?;
    content_webviews.insert(initial_site, initial_content);
    apply_content_memory_policy(&content_webviews, active_site);

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::Ipc { target, body }) => {
                if target == IpcTarget::Shell {
                    match parse_shell_command(&body) {
                        Some(Ok(ShellCommand::SwitchSite(site))) => {
                            let result = switch_active_site(
                                site,
                                &window,
                                &mut web_context,
                                &mut content_webviews,
                                &mut active_site,
                                &settings,
                                &settings_json,
                                detected_proxy.as_ref(),
                                runtime_ready,
                                event_proxy.clone(),
                            );
                            if let Err(error) = result {
                                eprintln!("could not switch active site: {error:#}");
                            }
                            sync_shell_tabs(&shell_webview, active_site, &content_webviews);
                        }
                        Some(Ok(ShellCommand::CloseSite(site))) => {
                            if let Err(error) =
                                close_content_site(site, &mut content_webviews, &mut active_site)
                            {
                                eprintln!("could not close content site: {error:#}");
                            }
                            sync_shell_tabs(&shell_webview, active_site, &content_webviews);
                        }
                        Some(Ok(ShellCommand::OptimizeMemory)) => {
                            let released =
                                optimize_content_memory(&mut content_webviews, active_site);
                            eprintln!(
                                "memory optimization released {released} background webviews"
                            );
                            sync_shell_tabs(&shell_webview, active_site, &content_webviews);
                        }
                        Some(Ok(ShellCommand::NavBack)) => {
                            if let Some(webview) = content_webviews.get(&active_site)
                                && let Err(error) = webview.evaluate_script("history.back();")
                            {
                                eprintln!("could not go back: {error:#}");
                            }
                        }
                        Some(Ok(ShellCommand::NavForward)) => {
                            if let Some(webview) = content_webviews.get(&active_site)
                                && let Err(error) = webview.evaluate_script("history.forward();")
                            {
                                eprintln!("could not go forward: {error:#}");
                            }
                        }
                        Some(Ok(ShellCommand::ReloadActive)) => {
                            if let Some(webview) = content_webviews.get(&active_site)
                                && let Err(error) =
                                    webview.evaluate_script("window.location.reload();")
                            {
                                eprintln!("could not reload active page: {error:#}");
                            }
                        }
                        Some(Err(error)) => eprintln!("invalid shell IPC command: {error:#}"),
                        None => eprintln!("ignored unknown shell IPC command: {body}"),
                    }
                    return;
                }

                if let Some(id) = parse_memory_optimization_request_id(&body) {
                    let released = optimize_content_memory(&mut content_webviews, active_site);
                    sync_shell_tabs(&shell_webview, active_site, &content_webviews);
                    let response = ipc_ok(
                        id.as_deref(),
                        memory_optimization_payload(
                            released,
                            active_site,
                            loaded_sites(&content_webviews),
                        ),
                    );
                    let script = format!(
                        "window.__chatgptClientReceive && window.__chatgptClientReceive({});",
                        response
                    );
                    let result = match target {
                        IpcTarget::Shell => shell_webview.evaluate_script(&script),
                        IpcTarget::Site(site) => content_webviews
                            .get(&site)
                            .map(|webview| webview.evaluate_script(&script))
                            .unwrap_or(Ok(())),
                    };
                    if let Err(error) = result {
                        eprintln!("could not send memory optimization response: {error:#}");
                    }
                    return;
                }

                let app_state = Arc::clone(&app_state);
                let event_proxy = event_proxy.clone();
                thread::spawn(move || {
                    let response = handle_ipc_message_concurrent(&body, &app_state);
                    let _ = event_proxy.send_event(UserEvent::IpcResponse { target, response });
                });
            }
            Event::UserEvent(UserEvent::IpcResponse { target, response }) => {
                let script = format!(
                    "window.__chatgptClientReceive && window.__chatgptClientReceive({});",
                    response
                );
                let result = match target {
                    IpcTarget::Shell => shell_webview.evaluate_script(&script),
                    IpcTarget::Site(site) => content_webviews
                        .get(&site)
                        .map(|webview| webview.evaluate_script(&script))
                        .unwrap_or(Ok(())),
                };
                if let Err(error) = result {
                    eprintln!("could not send IPC response to target webview: {error:#}");
                }
            }
            Event::UserEvent(UserEvent::RuntimeReady) => {
                runtime_ready = true;
                for (site, webview) in &content_webviews {
                    if let Err(error) =
                        webview.evaluate_script(&runtime_ready_script_for_site(*site))
                    {
                        eprintln!("could not notify {site:?} that runtime is ready: {error:#}");
                    }
                }
            }
            Event::LoopDestroyed => {
                stop_runtime(&app_state);
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                let (width, height) = logical_window_size(&window);
                if let Err(error) = shell_webview.set_bounds(top_bar_bounds(width)) {
                    eprintln!("could not resize shell webview: {error:#}");
                }
                let bounds = content_bounds(width, height);
                for webview in content_webviews.values() {
                    if let Err(error) = webview.set_bounds(bounds) {
                        eprintln!("could not resize content webview: {error:#}");
                    }
                }
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                stop_runtime(&app_state);
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn stop_runtime(app_state: &Arc<Mutex<AppRuntimeState>>) {
    if let Ok(mut state) = app_state.lock()
        && let Some(runtime) = state.clash_runtime.as_mut()
    {
        runtime.stop();
    }
}

fn heal_startup_runtime_ports(settings: &mut AppSettings) -> Result<()> {
    if !matches!(settings.proxy.mode, ProxyMode::InternalClash) {
        return Ok(());
    }

    let ports = resolved_runtime_ports(settings)?;
    settings.proxy.mixed_port = ports.mixed_port;
    settings.proxy.controller_port = ports.controller_port;
    Ok(())
}

fn site_initial_url(settings: &AppSettings, site: AiSite, runtime_ready: bool) -> String {
    if matches!(settings.proxy.mode, ProxyMode::InternalClash) {
        if runtime_ready {
            return site.url().to_string();
        }

        waiting_page_url(site.title())
    } else {
        site.url().to_string()
    }
}

fn waiting_page_url(site_title: &str) -> String {
    let html = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>AI Web Client</title>
  <style>
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #fff; color: #111; font: 16px/1.5 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    main { width: min(520px, calc(100vw - 40px)); }
    h1 { margin: 0 0 10px; font-size: 24px; }
    p { margin: 6px 0; color: #555; }
  </style>
</head>
<body>
  <main>
    <h1>正在启动内置代理</h1>
    <p>代理就绪后会自动打开 __SITE_TITLE__。右下角“设”可以查看订阅、节点和日志。</p>
  </main>
</body>
</html>"#
        .replace("__SITE_TITLE__", site_title);
    format!("data:text/html;charset=utf-8,{}", encode(&html))
}

fn runtime_ready_script_for_site(site: AiSite) -> String {
    r#"
      const isWaitingPage = window.location.href.startsWith('data:text/html') || window.location.href === 'about:blank';
      if (isWaitingPage) {
        window.location.href = '__SITE_URL__';
      } else {
        window.__chatgptClientRuntimeReady && window.__chatgptClientRuntimeReady();
      }
    "#
    .replace("__SITE_URL__", site.url())
}

fn top_bar_bounds(width: f64) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
        size: wry::dpi::LogicalSize::new(width.max(0.0), TOP_BAR_HEIGHT).into(),
    }
}

fn content_bounds(width: f64, height: f64) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(0.0, TOP_BAR_HEIGHT).into(),
        size: wry::dpi::LogicalSize::new(width.max(0.0), (height - TOP_BAR_HEIGHT).max(0.0)).into(),
    }
}

fn logical_window_size(window: &Window) -> (f64, f64) {
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    (size.width, size.height)
}

fn build_shell_webview(
    window: &Window,
    web_context: &mut WebContext,
    active_site: AiSite,
    bounds: wry::Rect,
    detected_proxy: Option<&ProxySettings>,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<WebView> {
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_custom_protocol(SHELL_PROTOCOL.into(), move |_webview_id, _request| {
            top_shell_response(active_site)
        })
        .with_url(top_shell_url())
        .with_bounds(bounds)
        .with_ipc_handler(move |request| {
            let _ = event_proxy.send_event(UserEvent::Ipc {
                target: IpcTarget::Shell,
                body: request.body().to_string(),
            });
        });

    let builder = apply_proxy_config(builder, detected_proxy);

    #[cfg(windows)]
    let builder = apply_windows_diagnostics(builder, detected_proxy);

    builder
        .build_as_child(window)
        .context("could not build shell webview")
}

#[allow(clippy::too_many_arguments)]
fn build_content_webview(
    window: &Window,
    web_context: &mut WebContext,
    site: AiSite,
    bounds: wry::Rect,
    settings: &AppSettings,
    settings_json: &str,
    detected_proxy: Option<&ProxySettings>,
    runtime_ready: bool,
    visible: bool,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<WebView> {
    let target = IpcTarget::Site(site);
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(site_initial_url(settings, site, runtime_ready))
        .with_bounds(bounds)
        .with_visible(visible)
        .with_initialization_script(settings_button_script(settings_json))
        .with_ipc_handler(move |request| {
            let _ = event_proxy.send_event(UserEvent::Ipc {
                target,
                body: request.body().to_string(),
            });
        })
        .with_new_window_req_handler(|_url, _features| NewWindowResponse::Allow);

    let builder = apply_proxy_config(builder, detected_proxy);

    #[cfg(windows)]
    let builder = apply_windows_diagnostics(builder, detected_proxy);

    builder
        .build_as_child(window)
        .with_context(|| format!("could not build {} webview", site.title()))
}

#[allow(clippy::too_many_arguments)]
fn switch_active_site(
    site: AiSite,
    window: &Window,
    web_context: &mut WebContext,
    content_webviews: &mut HashMap<AiSite, WebView>,
    active_site: &mut AiSite,
    settings: &AppSettings,
    settings_json: &str,
    detected_proxy: Option<&ProxySettings>,
    runtime_ready: bool,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<()> {
    if let std::collections::hash_map::Entry::Vacant(entry) = content_webviews.entry(site) {
        let (width, height) = logical_window_size(window);
        let webview = build_content_webview(
            window,
            web_context,
            site,
            content_bounds(width, height),
            settings,
            settings_json,
            detected_proxy,
            runtime_ready,
            false,
            event_proxy,
        )?;
        entry.insert(webview);
    }

    if *active_site != site {
        if let Some(webview) = content_webviews.get(active_site) {
            webview.set_visible(false)?;
        }
        if let Some(webview) = content_webviews.get(&site) {
            webview.set_visible(true)?;
            let _ = webview.focus();
        }
        *active_site = site;
    }
    apply_content_memory_policy(content_webviews, *active_site);

    Ok(())
}

fn close_content_site(
    site: AiSite,
    content_webviews: &mut HashMap<AiSite, WebView>,
    active_site: &mut AiSite,
) -> Result<bool> {
    if site == AiSite::ChatGpt {
        apply_content_memory_policy(content_webviews, *active_site);
        return Ok(false);
    }

    if *active_site == site
        && let Some(chatgpt_webview) = content_webviews.get(&AiSite::ChatGpt)
    {
        chatgpt_webview.set_visible(true)?;
        set_content_memory_level(chatgpt_webview, ContentMemoryLevel::Active);
        let _ = chatgpt_webview.focus();
        *active_site = AiSite::ChatGpt;
    }

    let removed = content_webviews.remove(&site).is_some();
    request_content_garbage_collection(content_webviews);
    apply_content_memory_policy(content_webviews, *active_site);
    trim_current_process_working_set();

    Ok(removed)
}

fn optimize_content_memory(
    content_webviews: &mut HashMap<AiSite, WebView>,
    active_site: AiSite,
) -> usize {
    let loaded = loaded_sites(content_webviews);
    let releasable = releasable_sites_for_memory(active_site, &loaded);
    let mut released = 0;

    for site in releasable {
        if content_webviews.remove(&site).is_some() {
            released += 1;
        }
    }

    request_content_garbage_collection(content_webviews);
    apply_content_memory_policy(content_webviews, active_site);
    trim_current_process_working_set();

    released
}

fn releasable_sites_for_memory(active_site: AiSite, loaded_sites: &[AiSite]) -> Vec<AiSite> {
    loaded_sites
        .iter()
        .copied()
        .filter(|site| *site != AiSite::ChatGpt && *site != active_site)
        .collect()
}

fn loaded_sites(content_webviews: &HashMap<AiSite, WebView>) -> Vec<AiSite> {
    AiSite::all()
        .iter()
        .copied()
        .filter(|site| content_webviews.contains_key(site))
        .collect()
}

fn loaded_site_keys_from_sites(sites: &[AiSite]) -> Vec<&'static str> {
    sites.iter().map(|site| site.key()).collect()
}

fn memory_optimization_payload(released: usize, active_site: AiSite, loaded: Vec<AiSite>) -> Value {
    json!({
        "released_pages": released,
        "active_site": active_site.key(),
        "loaded_pages": loaded_site_keys_from_sites(&loaded),
    })
}

fn sync_shell_tabs(
    shell_webview: &WebView,
    active_site: AiSite,
    content_webviews: &HashMap<AiSite, WebView>,
) {
    let script = shell_tab_sync_script(active_site, &loaded_sites(content_webviews));
    if let Err(error) = shell_webview.evaluate_script(&script) {
        eprintln!("could not sync top tab state: {error:#}");
    }
}

fn shell_tab_sync_script(active_site: AiSite, loaded: &[AiSite]) -> String {
    format!(
        "window.__aiClientSyncTabs && window.__aiClientSyncTabs({}, {});",
        json!(active_site.key()),
        json!(loaded_site_keys_from_sites(loaded))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentMemoryLevel {
    Active,
    Background,
}

fn apply_content_memory_policy(content_webviews: &HashMap<AiSite, WebView>, active_site: AiSite) {
    for (site, webview) in content_webviews {
        let level = if *site == active_site {
            ContentMemoryLevel::Active
        } else {
            ContentMemoryLevel::Background
        };
        set_content_memory_level(webview, level);
    }
}

fn set_content_memory_level(webview: &WebView, level: ContentMemoryLevel) {
    #[cfg(windows)]
    {
        let target = match level {
            ContentMemoryLevel::Active => MemoryUsageLevel::Normal,
            ContentMemoryLevel::Background => MemoryUsageLevel::Low,
        };
        if let Err(error) = webview.set_memory_usage_level(target) {
            eprintln!("could not set WebView2 memory usage level: {error:#}");
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (webview, level);
    }
}

fn request_content_garbage_collection(content_webviews: &HashMap<AiSite, WebView>) {
    for webview in content_webviews.values() {
        request_webview_garbage_collection(webview);
    }
}

fn request_webview_garbage_collection(webview: &WebView) {
    let script = r#"
      try {
        if (typeof window.gc === 'function') {
          window.gc();
          setTimeout(() => window.gc(), 250);
        }
      } catch (_) {}
    "#;

    if let Err(error) = webview.evaluate_script(script) {
        eprintln!("could not request webview garbage collection: {error:#}");
    }
}

#[cfg(windows)]
fn trim_current_process_working_set() {
    unsafe {
        let handle = windows_sys::Win32::System::Threading::GetCurrentProcess();
        trim_process_working_set(handle);
    }

    for pid in child_process_ids(std::process::id()) {
        trim_process_working_set_by_pid(pid);
    }
}

#[cfg(windows)]
unsafe fn trim_process_working_set(handle: windows_sys::Win32::Foundation::HANDLE) {
    unsafe {
        let _ = windows_sys::Win32::System::Threading::SetProcessWorkingSetSize(
            handle,
            usize::MAX,
            usize::MAX,
        );
    }
}

#[cfg(windows)]
fn trim_process_working_set_by_pid(pid: u32) {
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    };

    unsafe {
        let handle = OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        );
        if handle.is_null() {
            return;
        }

        trim_process_working_set(handle);
        let _ = windows_sys::Win32::Foundation::CloseHandle(handle);
    }
}

#[cfg(windows)]
fn child_process_ids(root_pid: u32) -> Vec<u32> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }

        let mut entries = Vec::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                entries.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);

        let mut descendants = Vec::new();
        let mut stack = vec![root_pid];
        while let Some(parent_pid) = stack.pop() {
            for (pid, parent) in &entries {
                if *parent == parent_pid && !descendants.contains(pid) {
                    descendants.push(*pid);
                    stack.push(*pid);
                }
            }
        }

        descendants
    }
}

#[cfg(not(windows))]
fn trim_current_process_working_set() {}

fn top_shell_html(active_site: AiSite) -> String {
    let tabs = AiSite::all()
        .iter()
        .enumerate()
        .map(|(index, site)| {
            let close_button = if *site == AiSite::ChatGpt {
                String::new()
            } else {
                format!(
                    r#"<button class="tab-close" type="button" title="关闭 {}" aria-label="关闭 {}" data-action="closeSite" data-site="{}">×</button>"#,
                    site.title(),
                    site.title(),
                    site.key()
                )
            };
            format!(
                r#"<div class="tab" role="button" tabindex="0" data-site="{}" data-active="{}" data-started="{}">
          <span class="tab-dot"></span>
          <span class="tab-name">{}</span>
          <span class="tab-shortcut">{}</span>
          {}
        </div>"#,
                site.key(),
                *site == active_site,
                *site == active_site,
                site.title(),
                index + 1,
                close_button
            )
        })
        .collect::<Vec<_>>()
        .join("\n        ");

    let html = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <style>
    :root { --surface: #fff; --surface-muted: #f0f2f5; --line: #d9dde3; --line-strong: #a8b0bd; --text: #15181d; --muted: #687080; --ok: #0b7a3b; }
    * { box-sizing: border-box; }
    html, body { width: 100%; height: 100%; margin: 0; overflow: hidden; background: transparent; color: var(--text); font: 13px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
    button { font: inherit; }
    .topbar { display: grid; grid-template-columns: minmax(0, 1fr) auto; align-items: center; gap: 12px; height: 52px; border-bottom: 1px solid var(--line); background: rgba(255,255,255,.96); padding: 8px 12px; }
    .tabs { display: flex; align-items: center; gap: 7px; min-width: 0; overflow: auto; scrollbar-width: none; }
    .tabs::-webkit-scrollbar { display: none; }
    .tab { display: inline-grid; grid-template-columns: auto minmax(0, auto) auto auto; align-items: center; gap: 7px; min-width: 116px; height: 36px; border: 1px solid transparent; border-radius: 8px; background: transparent; color: var(--muted); padding: 0 7px 0 10px; cursor: pointer; white-space: nowrap; }
    .tab:hover { background: var(--surface-muted); color: var(--text); }
    .tab[data-active="true"] { border-color: var(--line); background: var(--surface); color: var(--text); box-shadow: 0 4px 14px rgba(22,31,45,.08); }
    .tab-dot { width: 8px; height: 8px; border-radius: 999px; background: var(--line-strong); }
    .tab[data-started="true"] .tab-dot { background: var(--ok); }
    .tab-name { overflow: hidden; text-overflow: ellipsis; }
    .tab-shortcut { color: var(--muted); font-size: 11px; }
    .tab-close { display: grid; place-items: center; width: 22px; height: 22px; border: 0; border-radius: 999px; background: transparent; color: var(--muted); cursor: pointer; font-size: 17px; line-height: 1; }
    .tab-close:hover { background: #e5e9ef; color: var(--text); }
    .tab[data-started="false"] .tab-close { display: none; }
    .actions { display: flex; align-items: center; gap: 6px; min-width: 0; }
    .top-status { display: flex; align-items: center; gap: 7px; padding-right: 4px; }
    .pill { display: inline-flex; align-items: center; gap: 6px; min-height: 28px; border: 1px solid var(--line); border-radius: 999px; background: rgba(255,255,255,.76); padding: 0 10px; color: var(--muted); white-space: nowrap; }
    .pill strong { color: var(--text); font-weight: 650; }
    .toolbar-divider { width: 1px; height: 24px; background: var(--line); margin: 0 4px; }
    .icon-btn { display: grid; place-items: center; width: 34px; height: 34px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); color: var(--text); cursor: pointer; }
    .icon-btn:hover { border-color: var(--line-strong); background: var(--surface-muted); }
    .icon-btn svg { width: 17px; height: 17px; stroke: currentColor; stroke-width: 2; fill: none; stroke-linecap: round; stroke-linejoin: round; }
    @media (max-width: 820px) {
      .topbar { grid-template-columns: minmax(0, 1fr) auto; gap: 8px; padding-inline: 8px; }
      .tab { min-width: 94px; padding-inline: 8px; }
      .tab-shortcut, .top-status, .toolbar-divider { display: none; }
    }
  </style>
</head>
<body>
  <header class="topbar">
    <nav class="tabs" aria-label="AI pages">
      __TABS__
    </nav>
    <div class="actions" aria-label="Page actions">
      <div class="top-status" aria-label="Current status">
        <span class="pill"><strong>已登录</strong></span>
        <span class="pill"><strong>代理可用</strong></span>
        <span class="pill"><strong>页面常驻</strong></span>
      </div>
      <span class="toolbar-divider" aria-hidden="true"></span>
      <button class="icon-btn" type="button" title="后退" aria-label="后退" data-action="navBack"><svg viewBox="0 0 24 24"><path d="M15 18l-6-6 6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="前进" aria-label="前进" data-action="navForward"><svg viewBox="0 0 24 24"><path d="M9 18l6-6-6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="刷新" aria-label="刷新" data-action="reloadActive"><svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-2.64-6.36"></path><path d="M21 4v6h-6"></path></svg></button>
      <button class="icon-btn" type="button" title="清理内存" aria-label="清理内存" data-action="optimizeMemory"><svg viewBox="0 0 24 24"><path d="M3 17h18"></path><path d="M5 17l2-10h10l2 10"></path><path d="M8 21h8"></path><path d="M9 7V3h6v4"></path></svg></button>
    </div>
  </header>
  <script>
    const started = new Set(['__ACTIVE_SITE__']);
    let activeSite = '__ACTIVE_SITE__';
    let requestSeq = 0;
    function sendCommand(type, payload = {}) {
      if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
      window.ipc.postMessage(JSON.stringify({ id: String(++requestSeq), type, payload }));
    }
    function setActive(site) {
      activeSite = site;
      started.add(site);
      document.querySelectorAll('.tab').forEach(tab => {
        const active = tab.dataset.site === site;
        tab.dataset.active = String(active);
        tab.dataset.started = String(started.has(tab.dataset.site));
      });
    }
    function closeSite(site) {
      if (site === 'chatgpt') return;
      started.delete(site);
      if (activeSite === site) activeSite = 'chatgpt';
      setActive(activeSite);
      sendCommand('closeSite', { site });
    }
    function optimizeMemory() {
      const keep = new Set(['chatgpt', activeSite]);
      [...started].forEach(site => {
        if (!keep.has(site)) started.delete(site);
      });
      setActive(activeSite);
      sendCommand('optimizeMemory');
    }
    window.__aiClientSyncTabs = (active, loaded) => {
      started.clear();
      (Array.isArray(loaded) ? loaded : ['chatgpt']).forEach(site => started.add(site));
      started.add('chatgpt');
      activeSite = active || 'chatgpt';
      setActive(activeSite);
    };
    document.querySelectorAll('.tab').forEach(tab => {
      tab.addEventListener('click', event => {
        if (event.target.closest('.tab-close')) return;
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
      });
      tab.addEventListener('keydown', event => {
        if (!['Enter', ' '].includes(event.key)) return;
        event.preventDefault();
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
      });
    });
    document.querySelectorAll('.tab-close').forEach(button => {
      button.addEventListener('click', event => {
        event.stopPropagation();
        closeSite(button.dataset.site);
      });
    });
    document.querySelectorAll('.actions [data-action]').forEach(button => {
      button.addEventListener('click', () => {
        if (button.dataset.action === 'optimizeMemory') optimizeMemory();
        else sendCommand(button.dataset.action);
      });
    });
    document.addEventListener('keydown', event => {
      if (event.altKey && ['1', '2', '3', '4'].includes(event.key)) {
        const site = ['chatgpt', 'gemini', 'notebooklm', 'aistudio'][Number(event.key) - 1];
        setActive(site);
        sendCommand('switchSite', { site });
      }
    });
  </script>
</body>
</html>"#;

    html.replace("__TABS__", &tabs)
        .replace("__ACTIVE_SITE__", active_site.key())
}

fn top_shell_url() -> &'static str {
    SHELL_URL
}

fn top_shell_response(active_site: AiSite) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Cow::Owned(top_shell_html(active_site).into_bytes()))
        .expect("top shell response headers are valid")
}

fn spawn_runtime_watchdog(app_state: Arc<Mutex<AppRuntimeState>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(5));

            let Some(controller) = runtime_controller_snapshot(&app_state) else {
                continue;
            };
            let healthy = controller.proxy_state().is_ok();
            update_runtime_health(&app_state, healthy);
        }
    });
}

fn spawn_initial_runtime_start(
    app_state: Arc<Mutex<AppRuntimeState>>,
    event_proxy: tao::event_loop::EventLoopProxy<UserEvent>,
) {
    thread::spawn(move || {
        let Ok(mut state) = app_state.lock() else {
            return;
        };
        if matches!(state.settings.proxy.mode, ProxyMode::InternalClash)
            && state.restart_clash_runtime()
        {
            let _ = event_proxy.send_event(UserEvent::RuntimeReady);
        }
    });
}

fn runtime_controller_snapshot(
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Option<chatgpt_webview_client::controller::ClashController> {
    let Ok(state) = app_state.lock() else {
        return None;
    };
    if !matches!(state.settings.proxy.mode, ProxyMode::InternalClash) {
        return None;
    }

    state
        .clash_runtime
        .as_ref()
        .and_then(|runtime| runtime.controller().ok())
}

fn update_runtime_health(app_state: &Arc<Mutex<AppRuntimeState>>, healthy: bool) {
    let Ok(mut state) = app_state.lock() else {
        return;
    };
    if !matches!(state.settings.proxy.mode, ProxyMode::InternalClash) {
        state.last_health = RuntimeHealth::Stopped;
        return;
    }

    if healthy {
        state.runtime_error = None;
        state.last_health = RuntimeHealth::Running;
    } else if !state.restart_clash_runtime() {
        eprintln!(
            "could not restart mihomo runtime from watchdog: {}",
            state.runtime_error.as_deref().unwrap_or("unknown error")
        );
    }
}

#[cfg(windows)]
struct SingleInstanceGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn acquire_single_instance() -> Result<SingleInstanceGuard> {
    use windows_sys::Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
        System::Threading::CreateMutexW,
    };

    let name = wide_null(APP_MUTEX_NAME);
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        bail_with_last_os_error("could not create application single-instance lock")?;
    }

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
        anyhow::bail!(
            "ChatGPT client is already running. Please close the existing window before starting it again."
        );
    }

    Ok(SingleInstanceGuard(handle))
}

#[cfg(not(windows))]
fn acquire_single_instance() -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn bail_with_last_os_error(message: &str) -> Result<()> {
    Err(anyhow::anyhow!(
        "{message}: {}",
        std::io::Error::last_os_error()
    ))
}

fn handle_ipc_message_concurrent(body: &str, app_state: &Arc<Mutex<AppRuntimeState>>) -> String {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(message) => message,
        Err(error) => return ipc_error(None, format!("ignored invalid IPC message: {error}")),
    };
    let id = ipc_message_id(&message);
    let command = ipc_command(&message);

    let result = match command {
        "testDelay" => test_delay_payload_concurrent(&message, app_state),
        "testAllDelays" => test_all_delays_payload_concurrent(&message, app_state),
        "checkChatGpt" => check_chatgpt_payload_concurrent(app_state),
        "readProxyLogs" => read_proxy_logs_payload(),
        _ => match app_state.lock() {
            Ok(mut state) => handle_ipc_value(&message, &mut state),
            Err(error) => Err(anyhow::anyhow!("runtime state lock failed: {error}")),
        },
    };

    ipc_result(id.as_deref(), result)
}

fn handle_ipc_value(message: &Value, state: &mut AppRuntimeState) -> Result<Value> {
    let command = ipc_command(message);

    match command {
        "getProxyState" | "listProxyGroups" => resilient_proxy_state_payload(state),
        "saveSettings" => save_settings_payload(message, state),
        "selectProxy" => select_proxy_payload(message, state),
        "testDelay" => test_delay_payload(message, state),
        "testAllDelays" => test_all_delays_payload(message, state),
        "checkChatGpt" => check_chatgpt_payload(state),
        "restartRuntime" => restart_runtime_payload(state),
        "repairProxy" => repair_proxy_payload(state),
        "readProxyLogs" => read_proxy_logs_payload(),
        "refreshSubscription" => refresh_subscription_payload(state),
        _ => Err(anyhow::anyhow!("unknown IPC command: {command}")),
    }
}

fn ipc_result(id: Option<&str>, result: Result<Value>) -> String {
    match result {
        Ok(data) => ipc_ok(id, data),
        Err(error) => ipc_error(id, format!("{error:#}")),
    }
}

fn ipc_message_id(message: &Value) -> Option<String> {
    message
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn ipc_command(message: &Value) -> &str {
    message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn proxy_state_payload(state: &AppRuntimeState) -> Result<Value> {
    let mut controller_error = state.runtime_error.clone();
    let proxy_state = match state.clash_runtime.as_ref() {
        Some(runtime) => match runtime.controller()?.proxy_state() {
            Ok(proxy_state) => Some(serde_json::to_value(proxy_state)?),
            Err(error) => {
                controller_error = Some(format!("{error:#}"));
                None
            }
        },
        None => None,
    };

    Ok(json!({
        "settings": state.settings,
        "runtime_running": state.clash_runtime.is_some(),
        "runtime_health": state.health_label(),
        "runtime_error": state.runtime_error,
        "mixed_port": state.clash_runtime.as_ref().map(ClashRuntime::mixed_port),
        "controller_port": state.clash_runtime.as_ref().map(ClashRuntime::controller_port),
        "proxy_state": proxy_state,
        "controller_error": controller_error,
        "logs": read_mihomo_log_tail(80).unwrap_or_default(),
    }))
}

fn resilient_proxy_state_payload(state: &mut AppRuntimeState) -> Result<Value> {
    let payload = proxy_state_payload(state)?;
    if payload
        .get("controller_error")
        .is_some_and(|error| !error.is_null())
        && matches!(state.settings.proxy.mode, ProxyMode::InternalClash)
    {
        state.restart_clash_runtime();
        return proxy_state_payload(state);
    }

    Ok(payload)
}

fn save_settings_payload(message: &Value, state: &mut AppRuntimeState) -> Result<Value> {
    let settings_value = message
        .get("payload")
        .and_then(|payload| payload.get("settings"))
        .or_else(|| message.get("settings"))
        .context("settings payload is missing")?;
    let settings = serde_json::from_value::<AppSettings>(settings_value.clone())
        .context("could not parse settings payload")?;
    let previous_active = state.settings.proxy.active_subscription_id.clone();
    let mut settings = settings.normalized();
    settings
        .proxy
        .normalize_subscriptions_with_previous_active(Some(&previous_active));

    save_settings(&settings)?;
    let restart_required = settings.proxy.mode != state.settings.proxy.mode
        || settings.proxy.active_subscription_url()
            != state.settings.proxy.active_subscription_url()
        || settings.proxy.active_subscription_id != state.settings.proxy.active_subscription_id
        || settings.proxy.auto_update_subscription != state.settings.proxy.auto_update_subscription;
    state.settings = settings;
    if restart_required {
        state.restart_clash_runtime();
    }

    Ok(json!({ "restart_required": false, "restarted": restart_required }))
}

fn select_proxy_payload(message: &Value, state: &mut AppRuntimeState) -> Result<Value> {
    state.ensure_clash_runtime()?;
    let payload = message
        .get("payload")
        .context("selectProxy payload is missing")?;
    let group = payload
        .get("group")
        .and_then(Value::as_str)
        .context("group is missing")?;
    let proxy = payload
        .get("proxy")
        .and_then(Value::as_str)
        .context("proxy is missing")?;
    let runtime = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?;

    runtime.controller()?.select_proxy(group, proxy)?;
    state.settings.proxy.selected_group = group.to_string();
    state.settings.proxy.selected_proxy = proxy.to_string();
    save_settings(&state.settings)?;

    Ok(json!({ "selected_group": group, "selected_proxy": proxy }))
}

fn test_delay_payload(message: &Value, state: &mut AppRuntimeState) -> Result<Value> {
    state.ensure_clash_runtime()?;
    let payload = message
        .get("payload")
        .context("testDelay payload is missing")?;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .context("proxy name is missing")?;
    let runtime = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?;
    let result = runtime
        .controller()?
        .test_delay(name, DELAY_TEST_URL, DELAY_TIMEOUT_MS);

    Ok(serde_json::to_value(result)?)
}

fn test_delay_payload_concurrent(
    message: &Value,
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Result<Value> {
    let payload = message
        .get("payload")
        .context("testDelay payload is missing")?;
    let name = payload
        .get("name")
        .and_then(Value::as_str)
        .context("proxy name is missing")?
        .to_string();
    let controller = controller_snapshot(app_state)?;
    let result = controller.test_delay(&name, DELAY_TEST_URL, DELAY_TIMEOUT_MS);

    Ok(serde_json::to_value(result)?)
}

fn test_all_delays_payload(message: &Value, state: &mut AppRuntimeState) -> Result<Value> {
    state.ensure_clash_runtime()?;
    let payload = message.get("payload").cloned().unwrap_or_else(|| json!({}));
    let group = payload
        .get("group")
        .and_then(Value::as_str)
        .or_else(|| {
            let selected = state.settings.proxy.selected_group.trim();
            (!selected.is_empty()).then_some(selected)
        })
        .context("group is missing")?;
    let runtime = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?;
    let controller = runtime.controller()?;
    let proxy_state = controller.proxy_state()?;
    let group = proxy_state
        .groups
        .iter()
        .find(|candidate| candidate.name == group)
        .context("selected group was not found")?;
    let results = delay_results_for_group(controller, group);

    Ok(serde_json::to_value(results)?)
}

fn test_all_delays_payload_concurrent(
    message: &Value,
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Result<Value> {
    let payload = message.get("payload").cloned().unwrap_or_else(|| json!({}));
    let requested_group = payload
        .get("group")
        .and_then(Value::as_str)
        .map(str::to_string);
    let (controller, saved_group) = controller_and_selected_group_snapshot(app_state)?;
    let group_name = requested_group
        .or_else(|| (!saved_group.trim().is_empty()).then_some(saved_group))
        .context("group is missing")?;
    let proxy_state = controller.proxy_state()?;
    let group = proxy_state
        .groups
        .iter()
        .find(|candidate| candidate.name == group_name)
        .context("selected group was not found")?;
    let results = delay_results_for_group(controller, group);

    Ok(serde_json::to_value(results)?)
}

fn delay_results_for_group(
    controller: chatgpt_webview_client::controller::ClashController,
    group: &ProxyGroup,
) -> Vec<chatgpt_webview_client::controller::DelayResult> {
    group
        .nodes
        .chunks(4)
        .flat_map(|chunk| {
            let handles = chunk
                .iter()
                .map(|node| {
                    let controller = controller.clone();
                    let name = node.name.clone();
                    thread::spawn(move || {
                        controller.test_delay(&name, DELAY_TEST_URL, DELAY_TIMEOUT_MS)
                    })
                })
                .collect::<Vec<_>>();

            handles
                .into_iter()
                .map(|handle| match handle.join() {
                    Ok(result) => result,
                    Err(_) => chatgpt_webview_client::controller::DelayResult {
                        name: "unknown".to_string(),
                        delay_ms: None,
                        error: Some("delay test worker panicked".to_string()),
                    },
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
}

fn read_proxy_logs_payload() -> Result<Value> {
    Ok(json!({ "logs": read_mihomo_log_tail(160).unwrap_or_default() }))
}

fn check_chatgpt_payload(state: &mut AppRuntimeState) -> Result<Value> {
    state.ensure_clash_runtime()?;
    let name = if !state.settings.proxy.selected_group.trim().is_empty() {
        state.settings.proxy.selected_group.trim().to_string()
    } else if !state.settings.proxy.selected_proxy.trim().is_empty() {
        state.settings.proxy.selected_proxy.trim().to_string()
    } else {
        "GLOBAL".to_string()
    };
    let runtime = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?;
    let result = runtime
        .controller()?
        .test_delay(&name, DELAY_TEST_URL, DELAY_TIMEOUT_MS);

    Ok(json!({
        "ok": result.error.is_none(),
        "name": name,
        "delay_ms": result.delay_ms,
        "error": result.error,
    }))
}

fn check_chatgpt_payload_concurrent(app_state: &Arc<Mutex<AppRuntimeState>>) -> Result<Value> {
    let (controller, name) = controller_and_chatgpt_check_name_snapshot(app_state)?;
    let result = controller.test_delay(&name, DELAY_TEST_URL, DELAY_TIMEOUT_MS);

    Ok(json!({
        "ok": result.error.is_none(),
        "name": name,
        "delay_ms": result.delay_ms,
        "error": result.error,
    }))
}

fn controller_snapshot(
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Result<chatgpt_webview_client::controller::ClashController> {
    let mut state = app_state
        .lock()
        .map_err(|error| anyhow::anyhow!("runtime state lock failed: {error}"))?;
    state.ensure_clash_runtime()?;
    state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?
        .controller()
}

fn controller_and_selected_group_snapshot(
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Result<(chatgpt_webview_client::controller::ClashController, String)> {
    let mut state = app_state
        .lock()
        .map_err(|error| anyhow::anyhow!("runtime state lock failed: {error}"))?;
    state.ensure_clash_runtime()?;
    let selected_group = state.settings.proxy.selected_group.clone();
    let controller = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?
        .controller()?;

    Ok((controller, selected_group))
}

fn controller_and_chatgpt_check_name_snapshot(
    app_state: &Arc<Mutex<AppRuntimeState>>,
) -> Result<(chatgpt_webview_client::controller::ClashController, String)> {
    let mut state = app_state
        .lock()
        .map_err(|error| anyhow::anyhow!("runtime state lock failed: {error}"))?;
    state.ensure_clash_runtime()?;
    let name = if !state.settings.proxy.selected_group.trim().is_empty() {
        state.settings.proxy.selected_group.trim().to_string()
    } else if !state.settings.proxy.selected_proxy.trim().is_empty() {
        state.settings.proxy.selected_proxy.trim().to_string()
    } else {
        "GLOBAL".to_string()
    };
    let controller = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?
        .controller()?;

    Ok((controller, name))
}

fn restart_runtime_payload(state: &mut AppRuntimeState) -> Result<Value> {
    let restarted = state.restart_clash_runtime();

    Ok(json!({
        "restarted": restarted,
        "runtime_error": state.runtime_error,
        "runtime_health": state.health_label(),
    }))
}

fn repair_proxy_payload(state: &mut AppRuntimeState) -> Result<Value> {
    let result = state.repair_proxy()?;

    Ok(json!({
        "repaired": true,
        "group": result.group,
        "proxy": result.proxy,
        "delay_ms": result.delay_ms,
        "checked": result.checked,
        "runtime_health": state.health_label(),
    }))
}

fn refresh_subscription_payload(state: &mut AppRuntimeState) -> Result<Value> {
    state.ensure_clash_runtime()?;
    let runtime = state
        .clash_runtime
        .as_ref()
        .context("internal Clash runtime is not running")?;

    runtime.refresh_config(&state.settings)?;
    let selected = state.ensure_valid_proxy_selection()?;

    Ok(json!({
        "restart_required": false,
        "selected_group": selected.as_ref().map(|(group, _)| group.clone()),
        "selected_proxy": selected.as_ref().map(|(_, proxy)| proxy.clone()),
    }))
}

fn ordered_proxy_candidates(
    groups: &[ProxyGroup],
    saved_group: &str,
    saved_proxy: &str,
) -> Vec<(String, String)> {
    let mut candidates = Vec::new();

    let proxy_state = ProxyState {
        groups: groups.to_vec(),
    };
    if let Some(group) = preferred_proxy_group(&proxy_state, saved_group) {
        push_group_candidates(&mut candidates, group, saved_proxy);
    }

    for group in groups
        .iter()
        .filter(|group| !group.name.eq_ignore_ascii_case("GLOBAL"))
    {
        push_group_candidates(&mut candidates, group, saved_proxy);
    }
    for group in groups {
        push_group_candidates(&mut candidates, group, saved_proxy);
    }

    candidates.dedup();
    candidates
}

fn push_group_candidates(
    candidates: &mut Vec<(String, String)>,
    group: &ProxyGroup,
    saved_proxy: &str,
) {
    if let Some(node) = preferred_proxy_node(group, saved_proxy) {
        candidates.push((group.name.clone(), node.name.clone()));
    }
    for node in group
        .nodes
        .iter()
        .filter(|node| !node.name.eq_ignore_ascii_case("DIRECT"))
    {
        candidates.push((group.name.clone(), node.name.clone()));
    }
    for node in &group.nodes {
        candidates.push((group.name.clone(), node.name.clone()));
    }
}

fn ipc_ok(id: Option<&str>, data: Value) -> String {
    json!({ "id": id, "ok": true, "data": data }).to_string()
}

fn ipc_error(id: Option<&str>, error: String) -> String {
    json!({ "id": id, "ok": false, "error": error }).to_string()
}

fn parse_shell_command(body: &str) -> Option<Result<ShellCommand>> {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(message) => message,
        Err(error) => return Some(Err(anyhow::anyhow!("invalid shell IPC message: {error}"))),
    };
    let command = message
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match command {
        "switchSite" => {
            let site = message
                .get("payload")
                .and_then(|payload| payload.get("site"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(
                AiSite::from_key(site)
                    .map(ShellCommand::SwitchSite)
                    .ok_or_else(|| anyhow::anyhow!("unknown site: {site}")),
            )
        }
        "closeSite" => {
            let site = message
                .get("payload")
                .and_then(|payload| payload.get("site"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(
                AiSite::from_key(site)
                    .filter(|site| *site != AiSite::ChatGpt)
                    .map(ShellCommand::CloseSite)
                    .ok_or_else(|| anyhow::anyhow!("unknown closable site: {site}")),
            )
        }
        "optimizeMemory" => Some(Ok(ShellCommand::OptimizeMemory)),
        "navBack" => Some(Ok(ShellCommand::NavBack)),
        "navForward" => Some(Ok(ShellCommand::NavForward)),
        "reloadActive" => Some(Ok(ShellCommand::ReloadActive)),
        _ => None,
    }
}

fn parse_memory_optimization_request_id(body: &str) -> Option<Option<String>> {
    let message = serde_json::from_str::<Value>(body).ok()?;
    if ipc_command(&message) == "optimizeMemory" {
        Some(ipc_message_id(&message))
    } else {
        None
    }
}

fn settings_button_script(settings_json: &str) -> String {
    let script = r#"
(() => {
  const initialSettings = __SETTINGS__;
  const pending = new Map();
  const delays = new Map();
  let requestSeq = 0;
  let currentData = null;
  let delayRunToken = 0;
  let runtimeReadyPending = false;

  window.__chatgptClientReceive = (message) => {
    const entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id);
    if (message.ok) entry.resolve(message.data);
    else entry.reject(new Error(message.error || '操作失败'));
  };

  window.__chatgptClientRuntimeReady = () => { runtimeReadyPending = true; };

  function canUseNativeIpc() {
    return window.location.protocol !== 'data:'
      && window.location.protocol !== 'about:'
      && window.ipc
      && typeof window.ipc.postMessage === 'function';
  }

  function sendCommand(type, payload = {}) {
    if (!canUseNativeIpc()) {
      return Promise.reject(new Error('当前启动页暂不能读取设置，代理就绪后会自动打开 ChatGPT'));
    }
    const id = String(++requestSeq);
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, type, payload }));
      setTimeout(() => {
        if (pending.delete(id)) reject(new Error('操作超时'));
      }, 30000);
    });
  }

  function installSettingsButton() {
    if (document.getElementById('chatgpt-client-settings-button')) return;
    if (!document.body) return;

    const style = document.createElement('style');
    style.id = 'chatgpt-client-settings-style';
    style.textContent = `
      #chatgpt-client-settings-button {
        position: fixed; right: 20px; bottom: 92px; z-index: 2147483647;
        width: 40px; height: 40px; border: 1px solid rgba(0,0,0,.14);
        border-radius: 8px; background: rgba(255,255,255,.96); color: #111;
        font: 14px/1 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        box-shadow: 0 8px 24px rgba(0,0,0,.16); cursor: pointer;
      }
      #chatgpt-client-settings-panel {
        position: fixed; left: 50%; top: 50%; right: auto; bottom: auto; z-index: 2147483647;
        transform: translate(-50%, -50%);
        width: min(620px, calc(100vw - 32px)); max-height: min(780px, calc(100vh - 32px));
        overflow: hidden; border: 1px solid rgba(0,0,0,.14); border-radius: 8px;
        background: #fff; color: #111; box-shadow: 0 18px 52px rgba(0,0,0,.24);
        padding: 0; font: 13px/1.42 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
        display: flex; flex-direction: column;
      }
      #chatgpt-client-settings-panel[hidden] { display: none; }
      .cgpt-client-panel-head {
        display: flex; align-items: center; justify-content: space-between; gap: 12px;
        padding: 14px 14px 10px; border-bottom: 1px solid #ececec; flex: 0 0 auto;
      }
      .cgpt-client-panel-body { overflow: auto; padding: 12px 14px; flex: 1 1 auto; }
      .cgpt-client-panel-footer {
        display: flex; align-items: center; justify-content: space-between; gap: 12px;
        padding: 10px 14px 14px; border-top: 1px solid #ececec; background: #fff; flex: 0 0 auto;
      }
      #chatgpt-client-settings-panel h2 { margin: 0; font-size: 15px; font-weight: 650; }
      #chatgpt-client-settings-panel h3 { margin: 14px 0 8px; font-size: 13px; font-weight: 650; }
      #chatgpt-client-settings-panel label { display: grid; gap: 5px; margin: 8px 0; color: #333; }
      #chatgpt-client-settings-panel input, #chatgpt-client-settings-panel select {
        width: 100%; min-height: 32px; box-sizing: border-box; border: 1px solid #c9c9c9;
        border-radius: 6px; padding: 5px 8px; color: #111; background: #fff;
      }
      .cgpt-client-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
      .cgpt-client-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
      .cgpt-client-node-grid {
        display: grid; grid-template-columns: repeat(auto-fit, minmax(210px, 1fr)); gap: 8px; margin-top: 8px;
      }
      .cgpt-client-node-card {
        display: grid; gap: 6px; min-height: 92px; border: 1px solid #e1e1e1; border-radius: 8px;
        background: #fff; color: #111; padding: 9px; text-align: left;
      }
      .cgpt-client-node-card[data-current="true"] { border-color: #1a73e8; background: #eef5ff; }
      .cgpt-client-node-main { display: flex; align-items: flex-start; justify-content: space-between; gap: 8px; }
      .cgpt-client-node-name { white-space: normal; word-break: break-word; font-weight: 650; line-height: 1.25; }
      .cgpt-client-node-meta { color: #666; font-size: 12px; }
      .cgpt-client-node-actions { display: flex; align-items: center; justify-content: flex-end; gap: 6px; }
      .cgpt-client-node-delay { flex: 0 0 auto; font-weight: 650; }
      .cgpt-client-node-delay.good { color: #087a2e; }
      .cgpt-client-node-delay.warn { color: #9a6500; }
      .cgpt-client-node-delay.bad { color: #b42318; }
      .cgpt-client-node-empty {
        border: 1px solid #ececec; border-radius: 8px; background: #fafafa; padding: 14px; color: #555;
      }
      .cgpt-client-actions { display: flex; align-items: center; justify-content: flex-end; gap: 8px; margin-top: 10px; }
      .cgpt-client-btn {
        min-height: 32px; border: 1px solid #c9c9c9; border-radius: 6px; padding: 0 10px;
        background: #f7f7f7; color: #111; cursor: pointer;
      }
      .cgpt-client-icon-btn {
        width: 30px; height: 30px; border: 1px solid #d4d4d4; border-radius: 6px;
        background: #fff; color: #111; cursor: pointer; font-size: 18px; line-height: 1;
      }
      .cgpt-client-btn[data-primary="true"] { border-color: #111; background: #111; color: #fff; }
      .cgpt-client-btn:disabled { opacity: .55; cursor: default; }
      #chatgpt-client-settings-status { min-height: 18px; color: #555; font-size: 12px; flex: 1 1 auto; }
      .cgpt-client-pill { display: inline-flex; align-items: center; min-height: 24px; border: 1px solid #ddd; border-radius: 999px; padding: 0 8px; background: #fafafa; }
      .cgpt-client-delay.good { color: #087a2e; }
      .cgpt-client-delay.warn { color: #9a6500; }
      .cgpt-client-delay.bad { color: #b42318; }
      .cgpt-client-node-btn[data-current="true"] { border-color: #111; background: #111; color: #fff; }
      #chatgpt-client-log {
        margin-top: 8px; max-height: 130px; overflow: auto; white-space: pre-wrap; word-break: break-word;
        border: 1px solid #e1e1e1; border-radius: 6px; background: #fafafa; padding: 8px; color: #333; font-size: 12px;
      }
    `;
    document.documentElement.appendChild(style);

    const button = document.createElement('button');
    button.id = 'chatgpt-client-settings-button';
    button.type = 'button';
    button.title = '设置';
    button.setAttribute('aria-label', '设置');
    button.textContent = '设';

    const panel = document.createElement('section');
    panel.id = 'chatgpt-client-settings-panel';
    panel.hidden = true;
    panel.innerHTML = `
      <div class="cgpt-client-panel-head">
        <h2>快捷设置</h2>
        <button class="cgpt-client-icon-btn" type="button" data-action="close-panel" title="关闭设置" aria-label="关闭设置">×</button>
      </div>
      <div class="cgpt-client-panel-body">
        <div class="cgpt-client-row">
          <span class="cgpt-client-pill" data-view="runtime">状态：读取中</span>
          <span class="cgpt-client-pill" data-view="current">节点：未选择</span>
        </div>

        <h3>订阅与模式</h3>
        <label>代理模式
          <select data-field="mode">
            <option value="system">系统代理</option>
            <option value="direct">直连</option>
            <option value="internal_clash">内置 Clash</option>
          </select>
        </label>
        <label>当前订阅
          <select data-field="subscription_select"></select>
        </label>
        <div class="cgpt-client-grid">
          <label>当前订阅名称
            <input data-field="subscription_name" type="text" placeholder="机场 A" />
          </label>
          <label>当前订阅链接
            <input data-field="subscription_url" type="url" placeholder="https://..." />
          </label>
        </div>
        <div class="cgpt-client-actions">
          <button class="cgpt-client-btn" type="button" data-action="update-subscription">更新当前</button>
          <button class="cgpt-client-btn" type="button" data-action="delete-subscription">删除当前</button>
        </div>
        <div class="cgpt-client-grid">
          <label>新增订阅名称
            <input data-field="new_subscription_name" type="text" placeholder="机场 B" />
          </label>
          <label>新增订阅链接
            <input data-field="new_subscription_url" type="url" placeholder="https://..." />
          </label>
        </div>
        <div class="cgpt-client-actions">
          <button class="cgpt-client-btn" type="button" data-action="add-subscription">添加为新订阅</button>
        </div>
        <div class="cgpt-client-grid">
          <label>自动更新订阅
            <select data-field="auto_update_subscription">
              <option value="true">开启</option>
              <option value="false">关闭</option>
            </select>
          </label>
          <label>策略组
            <input data-field="selected_group" type="text" placeholder="PROXY" />
          </label>
        </div>
        <label>节点
          <input data-field="selected_proxy" type="text" placeholder="AUTO" />
        </label>
        <div class="cgpt-client-grid">
          <label>代理端口
            <input data-field="mixed_port" type="number" min="1024" max="65535" />
          </label>
          <label>控制端口
            <input data-field="controller_port" type="number" min="1024" max="65535" />
          </label>
        </div>
        <div class="cgpt-client-actions">
          <button class="cgpt-client-btn" type="button" data-action="refresh-subscription">刷新订阅</button>
        </div>

        <h3>节点选择</h3>
        <div class="cgpt-client-row">
          <select data-field="group_select"></select>
          <button class="cgpt-client-btn" type="button" data-action="reload">刷新列表</button>
          <button class="cgpt-client-btn" type="button" data-action="test-all">测速全部</button>
          <button class="cgpt-client-btn" type="button" data-action="test-fastest">最快节点</button>
          <button class="cgpt-client-btn" type="button" data-action="cancel-tests">停止测速</button>
          <button class="cgpt-client-btn" type="button" data-action="check-chatgpt">检测 ChatGPT</button>
          <button class="cgpt-client-btn" type="button" data-action="repair-proxy">一键修复</button>
          <button class="cgpt-client-btn" type="button" data-action="restart-runtime">重启代理</button>
          <button class="cgpt-client-btn" type="button" data-action="optimize-memory">清理内存</button>
        </div>
        <div class="cgpt-client-node-grid" data-view="nodes">
          <div class="cgpt-client-node-empty">暂无节点数据</div>
        </div>

        <h3>日志</h3>
        <div class="cgpt-client-actions">
          <button class="cgpt-client-btn" type="button" data-action="logs">刷新日志</button>
        </div>
        <pre id="chatgpt-client-log">暂无日志</pre>
      </div>
      <div class="cgpt-client-panel-footer">
        <div id="chatgpt-client-settings-status"></div>
        <div class="cgpt-client-actions">
          <button class="cgpt-client-btn" type="button" data-action="close-panel">关闭设置</button>
          <button class="cgpt-client-btn" type="button" data-primary="true" data-action="save">保存设置</button>
        </div>
      </div>
    `;

    document.body.append(button, panel);

    const fields = {
      mode: panel.querySelector('[data-field="mode"]'),
      subscription_select: panel.querySelector('[data-field="subscription_select"]'),
      subscription_name: panel.querySelector('[data-field="subscription_name"]'),
      subscription_url: panel.querySelector('[data-field="subscription_url"]'),
      new_subscription_name: panel.querySelector('[data-field="new_subscription_name"]'),
      new_subscription_url: panel.querySelector('[data-field="new_subscription_url"]'),
      auto_update_subscription: panel.querySelector('[data-field="auto_update_subscription"]'),
      selected_group: panel.querySelector('[data-field="selected_group"]'),
      selected_proxy: panel.querySelector('[data-field="selected_proxy"]'),
      mixed_port: panel.querySelector('[data-field="mixed_port"]'),
      controller_port: panel.querySelector('[data-field="controller_port"]'),
      group_select: panel.querySelector('[data-field="group_select"]'),
    };
    const views = {
      runtime: panel.querySelector('[data-view="runtime"]'),
      current: panel.querySelector('[data-view="current"]'),
      nodes: panel.querySelector('[data-view="nodes"]'),
      log: panel.querySelector('#chatgpt-client-log'),
      status: panel.querySelector('#chatgpt-client-settings-status'),
    };
    let subscriptions = [];
    let activeSubscriptionId = '';

    function setStatus(text) { views.status.textContent = text || ''; }
    function setSettings(settings) {
      fields.mode.value = settings.proxy.mode || 'system';
      subscriptions = normalizeSubscriptions(settings.proxy);
      activeSubscriptionId = settings.proxy.active_subscription_id || subscriptions[0]?.id || '';
      renderSubscriptions();
      fields.auto_update_subscription.value = String(settings.proxy.auto_update_subscription ?? true);
      fields.selected_group.value = settings.proxy.selected_group || '';
      fields.selected_proxy.value = settings.proxy.selected_proxy || '';
      fields.mixed_port.value = settings.proxy.mixed_port || 17898;
      fields.controller_port.value = settings.proxy.controller_port || 17899;
    }
    function readSettings() {
      syncSubscriptionEditor();
      const active = subscriptions.find(item => item.id === activeSubscriptionId);
      return {
        proxy: {
          mode: fields.mode.value,
          subscription_url: active?.url || fields.subscription_url.value.trim(),
          active_subscription_id: activeSubscriptionId,
          subscriptions,
          auto_update_subscription: fields.auto_update_subscription.value === 'true',
          selected_group: fields.selected_group.value.trim(),
          selected_proxy: fields.selected_proxy.value.trim(),
          mixed_port: Number(fields.mixed_port.value || 17898),
          controller_port: Number(fields.controller_port.value || 17899),
        }
      };
    }
    function normalizeSubscriptions(proxy) {
      const list = Array.isArray(proxy.subscriptions) ? proxy.subscriptions : [];
      const normalized = list
        .filter(item => item && String(item.url || '').trim())
        .map(item => ({
          id: item.id || makeSubscriptionId(item.url),
          name: item.name || '未命名订阅',
          url: String(item.url || '').trim(),
        }));
      if (!normalized.length && String(proxy.subscription_url || '').trim()) {
        normalized.push({
          id: makeSubscriptionId(proxy.subscription_url),
          name: '默认订阅',
          url: String(proxy.subscription_url).trim(),
        });
      }
      return normalized;
    }
    function makeSubscriptionId(url) {
      let hash = 2166136261;
      for (const ch of String(url || '')) {
        hash ^= ch.charCodeAt(0);
        hash = Math.imul(hash, 16777619) >>> 0;
      }
      return `sub-${hash.toString(16).padStart(8, '0')}`;
    }
    function renderSubscriptions() {
      if (!subscriptions.length) {
        activeSubscriptionId = '';
        fields.subscription_select.innerHTML = '<option value="">暂无订阅</option>';
        fields.subscription_name.value = '';
        fields.subscription_url.value = '';
        return;
      }
      if (!subscriptions.some(item => item.id === activeSubscriptionId)) {
        activeSubscriptionId = subscriptions[0].id;
      }
      fields.subscription_select.innerHTML = subscriptions
        .map(item => `<option value="${escapeAttr(item.id)}">${escapeHtml(item.name)} - ${escapeHtml(item.url)}</option>`)
        .join('');
      fields.subscription_select.value = activeSubscriptionId;
      const active = subscriptions.find(item => item.id === activeSubscriptionId);
      fields.subscription_name.value = active?.name || '';
      fields.subscription_url.value = active?.url || '';
    }
    function syncSubscriptionEditor() {
      const name = fields.subscription_name.value.trim();
      const url = fields.subscription_url.value.trim();
      if (!activeSubscriptionId || !url) return;
      const item = subscriptions.find(subscription => subscription.id === activeSubscriptionId);
      if (!item) return;
      item.name = name || item.name || '未命名订阅';
      item.url = url;
    }
    function addSubscriptionFromEditor() {
      const url = fields.new_subscription_url.value.trim();
      if (!url) throw new Error('请先填写订阅链接');
      const name = fields.new_subscription_name.value.trim() || `订阅 ${subscriptions.length + 1}`;
      const id = makeSubscriptionId(`${url}:${Date.now()}`);
      subscriptions.push({ id, name, url });
      activeSubscriptionId = id;
      fields.new_subscription_name.value = '';
      fields.new_subscription_url.value = '';
      renderSubscriptions();
    }
    function deleteActiveSubscription() {
      if (!activeSubscriptionId) return;
      subscriptions = subscriptions.filter(item => item.id !== activeSubscriptionId);
      activeSubscriptionId = subscriptions[0]?.id || '';
      renderSubscriptions();
    }
    function delayClass(value) {
      if (value == null) return 'bad';
      if (value < 500) return 'good';
      if (value < 1500) return 'warn';
      return 'bad';
    }
    function renderGroups() {
      const groups = currentData?.proxy_state?.groups || [];
      const savedGroup = fields.selected_group.value;
      const selected = groups.some(group => group.name === savedGroup) ? savedGroup : groups[0]?.name || '';
      fields.group_select.innerHTML = groups.map(group => `<option value="${escapeHtml(group.name)}">${escapeHtml(group.name)}</option>`).join('');
      fields.group_select.value = selected;
      renderNodes();
    }
    function renderNodes() {
      const groups = currentData?.proxy_state?.groups || [];
      const group = groups.find(item => item.name === fields.group_select.value);
      if (!group) {
        views.nodes.innerHTML = '<div class="cgpt-client-node-empty">内置 Clash 未运行或暂无策略组</div>';
        return;
      }
      fields.selected_group.value = group.name;
      fields.selected_proxy.value = group.selected || fields.selected_proxy.value || '';
      views.current.textContent = `节点：${group.name} / ${group.selected || '未选择'}`;
      views.nodes.innerHTML = group.nodes.map(node => {
        const delay = delays.get(node.name);
        const delayText = delay ? (delay.delay_ms == null ? '超时' : `${delay.delay_ms} ms`) : '-';
        const selected = node.name === group.selected ? '当前' : '选择';
        return `
          <article class="cgpt-client-node-card" data-current="${node.name === group.selected}" title="${escapeAttr(node.name)}">
            <div class="cgpt-client-node-main">
              <div class="cgpt-client-node-name">${escapeHtml(node.name)}</div>
              <div class="cgpt-client-node-delay ${delayClass(delay?.delay_ms)}">${escapeHtml(delayText)}</div>
            </div>
            <div class="cgpt-client-node-meta">${escapeHtml(node.kind)}</div>
            <div class="cgpt-client-node-actions">
              <button class="cgpt-client-btn cgpt-client-node-btn" type="button" data-current="${node.name === group.selected}" data-node="${escapeAttr(node.name)}" data-action="select-node">${selected}</button>
              <button class="cgpt-client-btn" type="button" data-node="${escapeAttr(node.name)}" data-action="test-node">测速</button>
            </div>
          </article>`;
      }).join('');
    }
    function renderState(data) {
      currentData = data;
      setSettings(data.settings || initialSettings);
      views.runtime.textContent = data.runtime_running ? '状态：内置 Clash 运行中' : '状态：内置 Clash 未运行';
      if (data.controller_error || data.runtime_error) setStatus(data.controller_error || data.runtime_error);
      views.log.textContent = data.logs || '暂无日志';
      renderGroups();
    }
    function currentGroup() {
      const groups = currentData?.proxy_state?.groups || [];
      return groups.find(item => item.name === fields.group_select.value);
    }
    async function runIncrementalDelayTests({ selectFastest = false } = {}) {
      const group = currentGroup();
      if (!group) throw new Error('请先选择策略组');
      const token = ++delayRunToken;
      const nodes = group.nodes.slice();
      let cursor = 0;
      let completed = 0;
      let best = null;
      setStatus(`正在测速 0/${nodes.length}...`);
      async function worker() {
        while (cursor < nodes.length && token === delayRunToken) {
          const node = nodes[cursor++];
          delays.set(node.name, { name: node.name, delay_ms: null, error: 'testing' });
          renderNodes();
          const result = await sendCommand('testDelay', { name: node.name });
          if (token !== delayRunToken) return;
          delays.set(result.name, result);
          if (result.delay_ms != null && (!best || result.delay_ms < best.delay_ms)) {
            best = { name: result.name, delay_ms: result.delay_ms };
          }
          completed += 1;
          renderNodes();
          setStatus(`正在测速 ${completed}/${nodes.length}...`);
        }
      }
      const workers = Array.from({ length: Math.min(4, nodes.length) }, () => worker());
      await Promise.all(workers);
      if (token !== delayRunToken) {
        setStatus('测速已停止');
        return null;
      }
      if (selectFastest && best) {
        await sendCommand('selectProxy', { group: group.name, proxy: best.name });
        fields.selected_group.value = group.name;
        fields.selected_proxy.value = best.name;
        await refreshState();
        setStatus(`已切换到最快节点：${best.name} · ${best.delay_ms} ms`);
      } else {
        setStatus('测速完成');
      }
      return best;
    }
    function escapeHtml(value) {
      return String(value ?? '').replace(/[&<>"']/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
    }
    function escapeAttr(value) { return escapeHtml(value); }

    async function refreshState() {
      setStatus('正在读取代理状态...');
      const data = await sendCommand('getProxyState');
      renderState(data);
      setStatus('');
    }
    window.__chatgptClientRuntimeReady = async () => {
      runtimeReadyPending = false;
      try {
        await refreshState();
        setStatus('内置代理已就绪');
      } catch (error) {
        setStatus(error.message);
      }
    };

    setSettings(initialSettings);
    if (runtimeReadyPending) window.__chatgptClientRuntimeReady();
    fields.subscription_select.addEventListener('change', () => {
      activeSubscriptionId = fields.subscription_select.value;
      renderSubscriptions();
    });
    button.addEventListener('click', async () => {
      panel.hidden = !panel.hidden;
      if (!panel.hidden) {
        try { await refreshState(); } catch (error) { setStatus(error.message); }
      }
    });
    fields.group_select.addEventListener('change', renderNodes);
    panel.addEventListener('click', async (event) => {
      const target = event.target.closest('[data-action]');
      if (!target) return;
      const action = target.dataset.action;
      try {
        if (action === 'save') {
          const result = await sendCommand('saveSettings', { settings: readSettings() });
          if (result.restarted) await refreshState();
          setStatus(result.restarted ? '已保存并重启内置代理' : '已保存');
        } else if (action === 'close-panel') {
          panel.hidden = true;
        } else if (action === 'add-subscription') {
          addSubscriptionFromEditor();
          const result = await sendCommand('saveSettings', { settings: readSettings() });
          if (result.restarted) await refreshState();
          setStatus(result.restarted ? '订阅已添加并重启内置代理' : '订阅已添加');
        } else if (action === 'update-subscription') {
          syncSubscriptionEditor();
          renderSubscriptions();
          const result = await sendCommand('saveSettings', { settings: readSettings() });
          if (result.restarted) await refreshState();
          setStatus(result.restarted ? '订阅已更新并重启内置代理' : '订阅已更新');
        } else if (action === 'delete-subscription') {
          deleteActiveSubscription();
          const result = await sendCommand('saveSettings', { settings: readSettings() });
          if (result.restarted) await refreshState();
          setStatus(result.restarted ? '订阅已删除并重启内置代理' : '订阅已删除');
        } else if (action === 'reload') {
          await refreshState();
        } else if (action === 'select-node') {
          const group = fields.group_select.value;
          const proxy = target.dataset.node;
          await sendCommand('selectProxy', { group, proxy });
          fields.selected_group.value = group;
          fields.selected_proxy.value = proxy;
          await refreshState();
          setStatus('节点已切换并保存');
        } else if (action === 'test-node') {
          const result = await sendCommand('testDelay', { name: target.dataset.node });
          delays.set(result.name, result);
          renderNodes();
        } else if (action === 'test-all') {
          await runIncrementalDelayTests();
        } else if (action === 'test-fastest') {
          await runIncrementalDelayTests({ selectFastest: true });
        } else if (action === 'cancel-tests') {
          delayRunToken += 1;
          setStatus('测速已停止');
        } else if (action === 'check-chatgpt') {
          setStatus('正在检测 ChatGPT 连通性...');
          const result = await sendCommand('checkChatGpt');
          setStatus(result.ok ? `ChatGPT 可访问，延时 ${result.delay_ms ?? '-'} ms` : `ChatGPT 检测失败：${result.error || '未知错误'}`);
        } else if (action === 'repair-proxy') {
          setStatus('正在一键修复代理...');
          const result = await sendCommand('repairProxy');
          await refreshState();
          setStatus(`修复完成：${result.group} / ${result.proxy} · ${result.delay_ms ?? '-'} ms`);
        } else if (action === 'restart-runtime') {
          setStatus('正在重启内置代理...');
          const result = await sendCommand('restartRuntime');
          await refreshState();
          setStatus(result.restarted ? '内置代理已重启' : `重启失败：${result.runtime_error || '未知错误'}`);
        } else if (action === 'optimize-memory') {
          setStatus('正在清理后台页面内存...');
          const result = await sendCommand('optimizeMemory');
          setStatus(`已清理后台页面：释放 ${result.released_pages ?? 0} 个页面`);
        } else if (action === 'logs') {
          const result = await sendCommand('readProxyLogs');
          views.log.textContent = result.logs || '暂无日志';
        } else if (action === 'refresh-subscription') {
          setStatus('正在刷新订阅...');
          const result = await sendCommand('refreshSubscription');
          await refreshState();
          setStatus(result.restart_required ? '订阅已刷新，重启后完全生效' : '订阅已刷新');
        }
      } catch (error) {
        setStatus(error.message);
      }
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', installSettingsButton, { once: true });
  } else {
    installSettingsButton();
  }
})();
"#;

    script.replace("__SETTINGS__", settings_json)
}

fn apply_proxy_config<'a>(
    builder: WebViewBuilder<'a>,
    proxy_settings: Option<&ProxySettings>,
) -> WebViewBuilder<'a> {
    let Some(proxy_settings) = proxy_settings else {
        return builder;
    };

    builder.with_proxy_config(to_wry_proxy_config(proxy_settings))
}

fn to_wry_proxy_config(proxy_settings: &ProxySettings) -> ProxyConfig {
    let endpoint = ProxyEndpoint {
        host: proxy_settings.host.clone(),
        port: proxy_settings.port.clone(),
    };

    match proxy_settings.scheme {
        ProxyScheme::Http => ProxyConfig::Http(endpoint),
        ProxyScheme::Socks5 => ProxyConfig::Socks5(endpoint),
    }
}

#[cfg(windows)]
fn apply_windows_diagnostics<'a>(
    builder: WebViewBuilder<'a>,
    proxy_settings: Option<&ProxySettings>,
) -> WebViewBuilder<'a> {
    let remote_debug_port = std::env::var("CHATGPT_CLIENT_REMOTE_DEBUG_PORT")
        .ok()
        .filter(|port| !port.trim().is_empty());
    let args = windows_browser_args(proxy_settings, remote_debug_port.as_deref());

    let builder = builder.with_additional_browser_args(args);
    if remote_debug_port.is_some() {
        builder.with_devtools(true)
    } else {
        builder
    }
}

#[cfg(windows)]
fn windows_browser_args(
    proxy_settings: Option<&ProxySettings>,
    remote_debug_port: Option<&str>,
) -> String {
    let mut args = String::from(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --js-flags=--expose-gc",
    );

    if let Some(port) = remote_debug_port {
        args.push_str(&format!(" --remote-debugging-port={port}"));
    }

    if let Some(proxy_settings) = proxy_settings {
        let scheme = match proxy_settings.scheme {
            ProxyScheme::Http => "http",
            ProxyScheme::Socks5 => "socks5",
        };
        args.push_str(&format!(
            " --proxy-server={scheme}://{}:{}",
            proxy_settings.host, proxy_settings.port
        ));
    }

    args
}

#[cfg(windows)]
fn show_startup_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    let title = wide_null("ChatGPT startup error");
    let message = wide_null(message);

    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_startup_error(_message: &str) {}

#[cfg(windows)]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        AiSite, ShellCommand, content_bounds, parse_shell_command, runtime_ready_script_for_site,
        settings_button_script, site_initial_url, top_shell_html, top_shell_url,
    };
    use chatgpt_webview_client::{AppSettings, CHATGPT_URL, ProxyMode};

    #[test]
    fn site_catalog_includes_all_top_tabs_in_order() {
        let keys = AiSite::all()
            .iter()
            .map(|site| site.key())
            .collect::<Vec<_>>();

        assert_eq!(keys, vec!["chatgpt", "gemini", "notebooklm", "aistudio"]);
    }

    #[test]
    fn site_initial_url_waits_for_internal_proxy_before_ready() {
        let mut settings = AppSettings::default();
        settings.proxy.mode = ProxyMode::InternalClash;

        let url = site_initial_url(&settings, AiSite::Gemini, false);

        assert!(url.starts_with("data:text/html"));
        assert!(url.contains("Gemini"));
    }

    #[test]
    fn runtime_ready_script_targets_each_site() {
        let script = runtime_ready_script_for_site(AiSite::NotebookLm);

        assert!(script.contains("window.location.href = 'https://notebooklm.google.com';"));
        assert!(!script.contains("window.location.href = 'https://chatgpt.com';"));
    }

    #[test]
    fn content_bounds_reserves_top_shell() {
        let bounds = content_bounds(1280.0, 900.0);

        assert_eq!(
            bounds.position,
            wry::dpi::LogicalPosition::new(0.0, 52.0).into()
        );
        assert_eq!(
            bounds.size,
            wry::dpi::LogicalSize::new(1280.0, 848.0).into()
        );
    }

    #[test]
    fn top_shell_html_contains_four_model_tabs() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert_eq!(html.matches("class=\"tab\"").count(), 4);
        assert!(html.contains("data-site=\"chatgpt\""));
        assert!(html.contains("data-site=\"gemini\""));
        assert!(html.contains("data-site=\"notebooklm\""));
        assert!(html.contains("data-site=\"aistudio\""));
    }

    #[test]
    fn top_shell_html_moves_status_pills_to_toolbar() {
        let html = top_shell_html(AiSite::Gemini);

        assert!(html.contains("class=\"top-status\""));
        assert!(html.contains("已登录"));
        assert!(html.contains("代理可用"));
        assert!(html.contains("页面常驻"));
        assert!(!html.contains("page-head"));
        assert!(!html.contains("browser-chrome"));
    }

    #[test]
    fn top_shell_html_posts_navigation_ipc_commands() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("switchSite"));
        assert!(html.contains("navBack"));
        assert!(html.contains("navForward"));
        assert!(html.contains("reloadActive"));
    }

    #[test]
    fn top_shell_html_exposes_close_buttons_for_secondary_tabs() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert_eq!(html.matches("class=\"tab-close\"").count(), 3);
        assert!(html.contains("data-action=\"closeSite\" data-site=\"gemini\""));
        assert!(html.contains("data-action=\"closeSite\" data-site=\"notebooklm\""));
        assert!(html.contains("data-action=\"closeSite\" data-site=\"aistudio\""));
        assert!(!html.contains("data-action=\"closeSite\" data-site=\"chatgpt\""));
    }

    #[test]
    fn top_shell_html_exposes_memory_optimization_action() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("data-action=\"optimizeMemory\""));
        assert!(html.contains("title=\"清理内存\""));
        assert!(html.contains("__aiClientSyncTabs"));
    }

    #[test]
    fn top_shell_loads_from_non_opaque_origin_for_ipc() {
        let url = top_shell_url();

        assert!(url.starts_with("aiclient://"));
        assert!(!url.starts_with("data:"));
        assert_ne!(url, "about:blank");
    }

    #[test]
    fn top_shell_keeps_fixed_height_on_narrow_windows() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(!html.contains("height: auto"));
        assert!(!html.contains("flex-wrap: wrap"));
        assert!(html.contains(".top-status, .toolbar-divider { display: none; }"));
    }

    #[test]
    fn parse_shell_command_recognizes_valid_site_switch() {
        let command =
            parse_shell_command(r#"{"id":"1","type":"switchSite","payload":{"site":"gemini"}}"#)
                .expect("shell command")
                .expect("valid shell command");

        assert_eq!(command, ShellCommand::SwitchSite(AiSite::Gemini));
    }

    #[test]
    fn parse_shell_command_recognizes_secondary_site_close() {
        let command =
            parse_shell_command(r#"{"id":"2","type":"closeSite","payload":{"site":"aistudio"}}"#)
                .expect("shell command")
                .expect("valid close command");

        assert_eq!(command, ShellCommand::CloseSite(AiSite::AiStudio));
    }

    #[test]
    fn parse_shell_command_recognizes_memory_optimization() {
        let command = parse_shell_command(r#"{"id":"3","type":"optimizeMemory"}"#)
            .expect("shell command")
            .expect("valid optimize command");

        assert_eq!(command, ShellCommand::OptimizeMemory);
    }

    #[test]
    fn parse_shell_command_rejects_invalid_site_key() {
        let error = parse_shell_command(r#"{"type":"switchSite","payload":{"site":"unknown"}}"#)
            .expect("shell command")
            .expect_err("invalid site should fail");

        assert!(error.to_string().contains("unknown site"));
    }

    #[test]
    fn parse_shell_command_ignores_existing_settings_ipc() {
        assert!(parse_shell_command(r#"{"id":"1","type":"getProxyState"}"#).is_none());
    }

    #[test]
    fn settings_panel_is_centered_with_footer_actions() {
        let script = settings_button_script(r#"{"proxy":{}}"#);

        assert!(script.contains("left: 50%; top: 50%;"));
        assert!(script.contains("transform: translate(-50%, -50%);"));
        assert!(script.contains("class=\"cgpt-client-panel-footer\""));
        assert!(script.contains("data-action=\"close-panel\""));
        assert!(script.contains(">保存设置</button>"));
    }

    #[test]
    fn settings_script_keeps_proxy_controls_without_old_site_switcher() {
        let script = settings_button_script(r#"{"proxy":{}}"#);

        assert!(!script.contains("chatgpt-client-status-chip"));
        assert!(!script.contains("data-view=\"quick_nodes\""));
        assert!(!script.contains("cgpt-client-site-switcher"));
        assert!(!script.contains("data-action=\"switch-site\""));
        assert!(script.contains("cgpt-client-node-grid"));
        assert!(script.contains("data-action=\"test-fastest\""));
        assert!(script.contains("data-action=\"cancel-tests\""));
        assert!(script.contains("data-action=\"check-chatgpt\""));
        assert!(script.contains("data-action=\"repair-proxy\""));
        assert!(script.contains("repairProxy"));
        assert!(script.contains("runIncrementalDelayTests"));
        assert!(script.contains("groups.some(group => group.name === savedGroup)"));
    }

    #[test]
    fn settings_script_exposes_memory_optimization_action() {
        let script = settings_button_script(r#"{"proxy":{}}"#);

        assert!(script.contains("data-action=\"optimize-memory\""));
        assert!(script.contains("optimizeMemory"));
        assert!(script.contains("后台页面"));
    }

    #[test]
    fn settings_script_uses_separate_fields_for_new_subscription() {
        let script = settings_button_script(r#"{"proxy":{}}"#);

        assert!(script.contains("data-field=\"new_subscription_name\""));
        assert!(script.contains("data-field=\"new_subscription_url\""));
        assert!(script.contains("fields.new_subscription_url.value.trim()"));
        assert!(script.contains("fields.new_subscription_url.value = '';"));
        assert!(script.contains(">添加为新订阅</button>"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_browser_args_keep_internal_proxy_when_diagnostics_are_enabled() {
        let proxy = chatgpt_webview_client::ProxySettings {
            scheme: chatgpt_webview_client::ProxyScheme::Http,
            host: "127.0.0.1".to_string(),
            port: "17898".to_string(),
        };

        let args = super::windows_browser_args(Some(&proxy), None);

        assert!(args.contains("--js-flags=--expose-gc"));
        assert!(args.contains("--proxy-server=http://127.0.0.1:17898"));
        assert!(!args.contains("--remote-debugging-port"));
    }

    #[test]
    fn memory_optimizer_releases_only_background_secondary_sites() {
        let loaded = vec![
            AiSite::ChatGpt,
            AiSite::Gemini,
            AiSite::NotebookLm,
            AiSite::AiStudio,
        ];

        assert_eq!(
            super::releasable_sites_for_memory(AiSite::ChatGpt, &loaded),
            vec![AiSite::Gemini, AiSite::NotebookLm, AiSite::AiStudio]
        );
        assert_eq!(
            super::releasable_sites_for_memory(AiSite::Gemini, &loaded),
            vec![AiSite::NotebookLm, AiSite::AiStudio]
        );
    }

    #[test]
    fn ordered_proxy_candidates_prefers_saved_valid_non_global_group() {
        use chatgpt_webview_client::controller::{ProxyGroup, ProxyNode};

        let groups = vec![
            ProxyGroup {
                name: "GLOBAL".to_string(),
                selected: "DIRECT".to_string(),
                nodes: vec![ProxyNode {
                    name: "DIRECT".to_string(),
                    kind: "Direct".to_string(),
                }],
            },
            ProxyGroup {
                name: "Proxy".to_string(),
                selected: "node-a".to_string(),
                nodes: vec![
                    ProxyNode {
                        name: "node-a".to_string(),
                        kind: "Vless".to_string(),
                    },
                    ProxyNode {
                        name: "DIRECT".to_string(),
                        kind: "Direct".to_string(),
                    },
                ],
            },
        ];

        let candidates = super::ordered_proxy_candidates(&groups, "Proxy", "node-a");

        assert_eq!(
            candidates.first(),
            Some(&(String::from("Proxy"), String::from("node-a")))
        );
    }

    #[test]
    fn settings_script_blocks_ipc_from_opaque_waiting_page() {
        let script = settings_button_script(r#"{"proxy":{}}"#);

        assert!(script.contains("function canUseNativeIpc()"));
        assert!(script.contains("window.location.protocol !== 'data:'"));
        assert!(script.contains("window.location.protocol !== 'about:'"));
        assert!(script.contains("当前启动页暂不能读取设置"));
    }

    #[test]
    fn runtime_ready_script_navigates_waiting_page_without_ipc() {
        let script = runtime_ready_script_for_site(AiSite::ChatGpt);

        assert!(script.contains("const isWaitingPage"));
        assert!(script.contains("if (isWaitingPage)"));
        assert!(script.contains("window.location.href = 'https://chatgpt.com';"));
        assert!(script.contains("} else {"));
        assert!(script.contains("window.__chatgptClientRuntimeReady"));
    }

    #[test]
    fn internal_clash_starts_with_local_waiting_page() {
        let mut settings = AppSettings::default();
        settings.proxy.mode = ProxyMode::InternalClash;

        let url = site_initial_url(&settings, AiSite::ChatGpt, false);

        assert!(url.starts_with("data:text/html"));
        assert!(
            url.contains(
                "%E6%AD%A3%E5%9C%A8%E5%90%AF%E5%8A%A8%E5%86%85%E7%BD%AE%E4%BB%A3%E7%90%86"
            )
        );
    }

    #[test]
    fn non_internal_clash_starts_at_chatgpt() {
        let settings = AppSettings::default();

        assert_eq!(
            site_initial_url(&settings, AiSite::ChatGpt, false),
            CHATGPT_URL
        );
    }
}
