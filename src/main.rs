#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chatgpt_webview_client::{
    AppSettings, CHATGPT_URL, DownloadSaveMode, DownloadSettings, ProxyMode, ProxyScheme,
    ProxySettings,
    clash::{ClashRuntime, read_mihomo_log_tail, resolved_runtime_ports, start_internal_clash},
    controller::{ProxyGroup, ProxyState, preferred_proxy_group, preferred_proxy_node},
    ensure_webview_profile_dir, load_settings, save_settings, startup_proxy,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tao::{
    dpi::LogicalSize,
    event::{Event, StartCause, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use urlencoding::encode;
use wry::{
    NewWindowResponse, ProxyConfig, ProxyEndpoint, WebContext, WebView, WebViewBuilder,
    http::{
        Response,
        header::{CACHE_CONTROL, CONTENT_TYPE, EXPIRES, PRAGMA},
    },
};

#[cfg(windows)]
use wry::{MemoryUsageLevel, WebViewBuilderExtWindows, WebViewExtWindows};

#[cfg(windows)]
use webview2_com::{
    CoTaskMemPWSTR,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT, ICoreWebView2_7, ICoreWebView2Environment6,
    },
    PrintToPdfCompletedHandler,
};

#[cfg(windows)]
use windows::core::Interface as _;

#[cfg(windows)]
const APP_MUTEX_NAME: &str = "Local\\ChatGPTWebviewClientSingleInstance";
const DELAY_TEST_URL: &str = "https://chatgpt.com/cdn-cgi/trace";
const DELAY_TIMEOUT_MS: u64 = 5000;
const TOP_BAR_HEIGHT: f64 = 52.0;
const SHELL_PROTOCOL: &str = "aiclient";
const SHELL_URL: &str = "aiclient://shell/index.html?v=download-manager-native-v2";
const DOWNLOAD_HISTORY_FILE_NAME: &str = "downloads.json";
const DEFAULT_MAX_DOWNLOAD_RECORDS: usize = 500;
static DOWNLOAD_IPC_TOKEN_SEQ: AtomicU64 = AtomicU64::new(1);

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

impl ExportFormat {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "markdown" | "md" => Some(Self::Markdown),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Pdf => "pdf",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Pdf => "pdf",
        }
    }
}

enum UserEvent {
    Ipc { target: IpcTarget, body: String },
    IpcResponse { target: IpcTarget, response: String },
    DownloadIpcResponse { target: IpcTarget, response: String },
    DownloadEvent(DownloadEvent),
    LatencyEvent(LatencyEvent),
    StartupProgress(StartupProgress),
    RuntimeReady,
    RuntimeFailed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupStage {
    LoadSettings,
    ResolvePorts,
    LoadSubscription,
    BuildMihomoConfig,
    StartMihomo,
    WaitController,
    RestoreNode,
    CheckConnectivity,
    Ready,
    Failed,
}

impl StartupStage {
    const ALL: [StartupStage; 10] = [
        StartupStage::LoadSettings,
        StartupStage::ResolvePorts,
        StartupStage::LoadSubscription,
        StartupStage::BuildMihomoConfig,
        StartupStage::StartMihomo,
        StartupStage::WaitController,
        StartupStage::RestoreNode,
        StartupStage::CheckConnectivity,
        StartupStage::Ready,
        StartupStage::Failed,
    ];

    fn all() -> &'static [StartupStage] {
        &Self::ALL
    }

    fn key(self) -> &'static str {
        match self {
            Self::LoadSettings => "load_settings",
            Self::ResolvePorts => "resolve_ports",
            Self::LoadSubscription => "load_subscription",
            Self::BuildMihomoConfig => "build_mihomo_config",
            Self::StartMihomo => "start_mihomo",
            Self::WaitController => "wait_controller",
            Self::RestoreNode => "restore_node",
            Self::CheckConnectivity => "check_connectivity",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::LoadSettings => "读取配置",
            Self::ResolvePorts => "检查本地端口",
            Self::LoadSubscription => "读取或更新订阅",
            Self::BuildMihomoConfig => "生成代理配置",
            Self::StartMihomo => "启动 mihomo",
            Self::WaitController => "等待控制器就绪",
            Self::RestoreNode => "恢复上次节点",
            Self::CheckConnectivity => "检测 ChatGPT 连通性",
            Self::Ready => "代理已就绪",
            Self::Failed => "代理启动失败",
        }
    }
}

#[derive(Debug, Clone)]
struct StartupProgress {
    stage: StartupStage,
    elapsed_secs: u64,
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadEventKind {
    Started,
    Completed,
    Failed,
    Diagnostic,
}

#[derive(Debug, Clone)]
struct DownloadEvent {
    kind: DownloadEventKind,
    status: String,
    path: Option<PathBuf>,
    url: Option<String>,
    bytes: Option<u64>,
    success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DownloadRecord {
    id: u64,
    filename: String,
    status: DownloadRecordStatus,
    path: Option<PathBuf>,
    url: Option<String>,
    bytes: Option<u64>,
    message: String,
    timestamp_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DownloadRecordStatus {
    Started,
    Completed,
    Failed,
    Diagnostic,
    Cancelled,
    Missing,
}

impl DownloadRecordStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Diagnostic => "diagnostic",
            Self::Cancelled => "cancelled",
            Self::Missing => "missing",
        }
    }
}

#[derive(Debug)]
struct DownloadHistory {
    next_id: u64,
    max_records: usize,
    records: Vec<DownloadRecord>,
}

impl Default for DownloadHistory {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_DOWNLOAD_RECORDS)
    }
}

impl DownloadHistory {
    fn new(max_records: usize) -> Self {
        Self {
            next_id: 0,
            max_records: max_records.max(1),
            records: Vec::new(),
        }
    }

    fn record(&mut self, event: DownloadEvent) {
        let status = match event.kind {
            DownloadEventKind::Started => DownloadRecordStatus::Started,
            DownloadEventKind::Completed if event.success => DownloadRecordStatus::Completed,
            DownloadEventKind::Completed | DownloadEventKind::Failed => {
                DownloadRecordStatus::Failed
            }
            DownloadEventKind::Diagnostic => DownloadRecordStatus::Diagnostic,
        };
        let filename = event
            .path
            .as_ref()
            .and_then(|path| cross_platform_file_name(path.as_os_str()))
            .or_else(|| {
                event
                    .url
                    .as_deref()
                    .and_then(|url| url.rsplit('/').next())
                    .filter(|name| !name.trim().is_empty())
            })
            .unwrap_or("download")
            .to_string();

        if matches!(
            event.kind,
            DownloadEventKind::Completed | DownloadEventKind::Failed
        ) && let Some(existing) = self.matching_active_record_mut(&event)
        {
            existing.status = status;
            existing.path = event.path;
            existing.url = event.url;
            existing.bytes = event.bytes;
            existing.message = event.status;
            existing.timestamp_ms = now_millis();
            return;
        }

        self.next_id += 1;
        self.records.insert(
            0,
            DownloadRecord {
                id: self.next_id,
                filename,
                status,
                path: event.path,
                url: event.url,
                bytes: event.bytes,
                message: event.status,
                timestamp_ms: now_millis(),
            },
        );
        self.records.truncate(self.max_records);
    }

    fn matching_active_record_mut(&mut self, event: &DownloadEvent) -> Option<&mut DownloadRecord> {
        self.records.iter_mut().find(|record| {
            record.status == DownloadRecordStatus::Started
                && ((event.path.is_some() && record.path == event.path)
                    || (event.url.is_some() && record.url == event.url))
        })
    }

    fn clear_completed(&mut self) {
        self.records
            .retain(|record| !matches!(record.status, DownloadRecordStatus::Completed));
    }

    fn delete_record(&mut self, id: u64) -> bool {
        let before = self.records.len();
        self.records.retain(|record| record.id != id);
        before != self.records.len()
    }

    fn payload(&self) -> Value {
        json!({
            "downloads": self.records.iter().map(download_record_payload).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct DownloadHistoryStore {
    version: u32,
    next_id: u64,
    records: Vec<DownloadRecord>,
}

fn download_record_payload(record: &DownloadRecord) -> Value {
    json!({
        "id": record.id,
        "filename": record.filename,
        "status": record.status.as_str(),
        "path": record.path.as_ref().map(|path| path.display().to_string()),
        "url": record.url,
        "bytes": record.bytes,
        "message": record.message,
        "timestamp_ms": record.timestamp_ms,
    })
}

fn download_history_path() -> Result<PathBuf> {
    Ok(chatgpt_webview_client::app_data_dir()?.join(DOWNLOAD_HISTORY_FILE_NAME))
}

fn load_download_history(max_records: usize) -> DownloadHistory {
    let Ok(path) = download_history_path() else {
        return DownloadHistory::new(max_records);
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return DownloadHistory::new(max_records);
    };
    let Ok(store) = serde_json::from_str::<DownloadHistoryStore>(&contents) else {
        eprintln!("could not parse download history at {}", path.display());
        return DownloadHistory::new(max_records);
    };

    download_history_from_store(store, max_records)
}

fn download_history_from_store(store: DownloadHistoryStore, max_records: usize) -> DownloadHistory {
    let mut records = store.records;
    for record in &mut records {
        if record.status == DownloadRecordStatus::Started {
            record.status = DownloadRecordStatus::Failed;
            record.message = "上次退出时下载未完成".to_string();
        }
    }
    records.truncate(max_records.max(1));

    DownloadHistory {
        next_id: store.next_id,
        max_records: max_records.max(1),
        records,
    }
}

fn save_download_history(history: &DownloadHistory) -> Result<()> {
    let path = download_history_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "could not create download history directory {}",
                parent.display()
            )
        })?;
    }

    let store = download_history_store(history);
    let contents =
        serde_json::to_vec_pretty(&store).context("could not serialize download history")?;
    std::fs::write(&path, contents)
        .with_context(|| format!("could not write download history to {}", path.display()))
}

fn download_history_store(history: &DownloadHistory) -> DownloadHistoryStore {
    DownloadHistoryStore {
        version: 1,
        next_id: history.next_id,
        records: history.records.clone(),
    }
}

fn persist_download_history(history: &DownloadHistory) {
    if let Err(error) = save_download_history(history) {
        eprintln!("could not save download history: {error:#}");
    }
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[derive(Debug, Clone)]
struct LatencyEvent {
    site: AiSite,
    delay_ms: Option<u64>,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum IpcTarget {
    Shell,
    DownloadManager,
    Site(AiSite),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportFormat {
    Markdown,
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellCommand {
    SwitchSite(AiSite),
    CloseSite(AiSite),
    OptimizeMemory,
    NavBack,
    NavForward,
    ReloadActive,
    OpenDownloadPath(PathBuf),
    OpenDownloadFolder(PathBuf),
    OpenDownloadManager,
    CloseDownloadManager,
    ClearCompletedDownloads,
    DeleteDownloadRecord(u64),
    OpenDownloadSettings,
    MeasureLatency(AiSite),
    ExportConversation(ExportFormat),
}

struct DownloadManagerWindow {
    window: Window,
    webview: WebView,
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
    #[cfg(windows)]
    let _single_instance = acquire_single_instance()?;
    #[cfg(not(windows))]
    acquire_single_instance()?;
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
    let mut download_history = load_download_history(settings.downloads.max_records);
    let mut download_manager_window: Option<DownloadManagerWindow> = None;
    let mut download_ipc_tokens = HashMap::new();
    let main_window_id = window.id();
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
    let initial_download_ipc_token = new_download_ipc_token(initial_site);
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
        &initial_download_ipc_token,
        event_proxy.clone(),
    )
    .context("could not create the initial content WebView2 instance")?;
    download_ipc_tokens.insert(initial_site, initial_download_ipc_token);
    content_webviews.insert(initial_site, initial_content);
    apply_content_memory_policy(&content_webviews, active_site);
    sync_downloads(&shell_webview, &download_history);
    let mut initial_runtime_start_spawned =
        !matches!(settings.proxy.mode, ProxyMode::InternalClash);

    event_loop.run(move |event, event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(StartCause::Init)
                if should_spawn_initial_runtime_start(
                    settings.proxy.mode.clone(),
                    initial_runtime_start_spawned,
                ) =>
            {
                initial_runtime_start_spawned = true;
                spawn_initial_runtime_start(Arc::clone(&app_state), event_proxy.clone());
            }
            Event::UserEvent(UserEvent::Ipc { target, body }) => {
                if matches!(target, IpcTarget::Shell | IpcTarget::DownloadManager) {
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
                                &mut download_ipc_tokens,
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
                        Some(Ok(ShellCommand::OpenDownloadPath(path))) => {
                            if let Err(error) = reveal_download_path(&path) {
                                let event = DownloadEvent {
                                    kind: DownloadEventKind::Failed,
                                    status: format!("打开文件位置失败：{error:#}"),
                                    path: Some(path),
                                    url: None,
                                    bytes: None,
                                    success: false,
                                };
                                let script = download_notification_script(
                                    &event.status,
                                    event.path.as_deref(),
                                    event.success,
                                );
                                let _ = shell_webview.evaluate_script(&script);
                            }
                        }
                        Some(Ok(ShellCommand::OpenDownloadFolder(path))) => {
                            if let Err(error) = reveal_download_folder(&path) {
                                let event = DownloadEvent {
                                    kind: DownloadEventKind::Failed,
                                    status: format!("打开下载目录失败：{error:#}"),
                                    path: Some(path),
                                    url: None,
                                    bytes: None,
                                    success: false,
                                };
                                let script = download_notification_script(
                                    &event.status,
                                    event.path.as_deref(),
                                    event.success,
                                );
                                let _ = shell_webview.evaluate_script(&script);
                            }
                        }
                        Some(Ok(ShellCommand::OpenDownloadManager)) => {
                            if let Err(error) = open_download_manager_window(
                                &mut download_manager_window,
                                event_loop,
                                event_proxy.clone(),
                                &download_history,
                            ) {
                                eprintln!("could not open download manager: {error:#}");
                                let event = DownloadEvent {
                                    kind: DownloadEventKind::Failed,
                                    status: format!("打开下载管理失败：{error:#}"),
                                    path: None,
                                    url: None,
                                    bytes: None,
                                    success: false,
                                };
                                let script = download_notification_script(
                                    &event.status,
                                    event.path.as_deref(),
                                    event.success,
                                );
                                let _ = shell_webview.evaluate_script(&script);
                            }
                        }
                        Some(Ok(ShellCommand::CloseDownloadManager)) => {
                            download_manager_window = None;
                        }
                        Some(Ok(ShellCommand::ClearCompletedDownloads)) => {
                            download_history.clear_completed();
                            persist_download_history(&download_history);
                            sync_downloads(&shell_webview, &download_history);
                            sync_download_manager_window(&download_manager_window, &download_history);
                        }
                        Some(Ok(ShellCommand::DeleteDownloadRecord(id))) => {
                            if download_history.delete_record(id) {
                                persist_download_history(&download_history);
                                sync_downloads(&shell_webview, &download_history);
                                sync_download_manager_window(
                                    &download_manager_window,
                                    &download_history,
                                );
                            }
                        }
                        Some(Ok(ShellCommand::OpenDownloadSettings)) => {
                            download_manager_window = None;
                            if let Some(webview) = content_webviews.get(&active_site)
                                && let Err(error) = webview.evaluate_script(
                                    "window.__chatgptClientOpenSettings ? window.__chatgptClientOpenSettings('downloads') : document.getElementById('chatgpt-client-settings-button')?.click();",
                                )
                            {
                                eprintln!("could not open download settings: {error:#}");
                            }
                        }
                        Some(Ok(ShellCommand::MeasureLatency(site))) => {
                            let app_state = Arc::clone(&app_state);
                            let event_proxy = event_proxy.clone();
                            thread::spawn(move || {
                                let event = measure_site_latency(site, &app_state);
                                let _ = event_proxy.send_event(UserEvent::LatencyEvent(event));
                            });
                        }
                        Some(Ok(ShellCommand::ExportConversation(format))) => {
                            if let Some(webview) = content_webviews.get(&active_site) {
                                let result = match format {
                                    ExportFormat::Markdown => webview
                                        .evaluate_script(&export_conversation_script(
                                            active_site,
                                            format,
                                        ))
                                        .map_err(anyhow::Error::from),
                                    ExportFormat::Pdf => {
                                        export_current_page_pdf(webview, active_site).map(|event| {
                                            let _ = event_proxy
                                                .send_event(UserEvent::DownloadEvent(event));
                                        })
                                    }
                                };

                                if let Err(error) = result {
                                    let event = DownloadEvent {
                                        kind: DownloadEventKind::Failed,
                                        status: format!("导出失败：{error:#}"),
                                        path: None,
                                        url: None,
                                        bytes: None,
                                        success: false,
                                    };
                                    let script = download_notification_script(
                                        &event.status,
                                        event.path.as_deref(),
                                        event.success,
                                    );
                                    let _ = shell_webview.evaluate_script(&script);
                                    download_history.record(event);
                                    persist_download_history(&download_history);
                                    sync_downloads(&shell_webview, &download_history);
                                }
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
                        IpcTarget::DownloadManager => download_manager_window
                            .as_ref()
                            .map(|manager| manager.webview.evaluate_script(&script))
                            .unwrap_or(Ok(())),
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

                if let Some(event) = parse_latency_event(&body) {
                    let _ = event_proxy.send_event(UserEvent::LatencyEvent(event));
                    return;
                }

                if is_save_download_request(&body) {
                    let event_proxy = event_proxy.clone();
                    thread::spawn(move || {
                        let (response, event) = handle_save_download_message_with_event(&body);
                        let _ = event_proxy
                            .send_event(UserEvent::DownloadIpcResponse { target, response });
                        let _ = event_proxy.send_event(UserEvent::DownloadEvent(event));
                    });
                    return;
                }

                if is_download_url_request(&body) {
                    let event_proxy = event_proxy.clone();
                    let proxy = latency_proxy_snapshot(&app_state);
                    let expected_token = match target {
                        IpcTarget::Site(site) => download_ipc_tokens.get(&site).cloned(),
                        IpcTarget::Shell | IpcTarget::DownloadManager => None,
                    };
                    thread::spawn(move || {
                        let (response, event) = handle_download_url_message_with_event(
                            &body,
                            proxy.as_ref(),
                            expected_token.as_deref(),
                        );
                        let _ = event_proxy
                            .send_event(UserEvent::DownloadIpcResponse { target, response });
                        let _ = event_proxy.send_event(UserEvent::DownloadEvent(event));
                    });
                    return;
                }

                if let Some(event) = parse_download_diagnostic_event(&body) {
                    let _ = event_proxy.send_event(UserEvent::DownloadEvent(event));
                    return;
                }

                if is_export_conversation_request(&body) {
                    let event_proxy = event_proxy.clone();
                    thread::spawn(move || {
                        let (response, event) =
                            handle_export_conversation_message_with_event(&body);
                        let _ = event_proxy
                            .send_event(UserEvent::DownloadIpcResponse { target, response });
                        let _ = event_proxy.send_event(UserEvent::DownloadEvent(event));
                    });
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
                    IpcTarget::DownloadManager => download_manager_window
                        .as_ref()
                        .map(|manager| manager.webview.evaluate_script(&script))
                        .unwrap_or(Ok(())),
                    IpcTarget::Site(site) => content_webviews
                        .get(&site)
                        .map(|webview| webview.evaluate_script(&script))
                        .unwrap_or(Ok(())),
                };
                if let Err(error) = result {
                    eprintln!("could not send IPC response to target webview: {error:#}");
                }
            }
            Event::UserEvent(UserEvent::DownloadIpcResponse { target, response }) => {
                let script = format!(
                    "window.__chatgptClientReceive && window.__chatgptClientReceive({});",
                    response
                );
                let result = match target {
                    IpcTarget::Shell => shell_webview.evaluate_script(&script),
                    IpcTarget::DownloadManager => download_manager_window
                        .as_ref()
                        .map(|manager| manager.webview.evaluate_script(&script))
                        .unwrap_or(Ok(())),
                    IpcTarget::Site(site) => content_webviews
                        .get(&site)
                        .map(|webview| webview.evaluate_script(&script))
                        .unwrap_or(Ok(())),
                };
                if let Err(error) = result {
                    eprintln!("could not send download IPC response: {error:#}");
                }
            }
            Event::UserEvent(UserEvent::DownloadEvent(event)) => {
                let last_download_dir = event
                    .path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map(|path| path.display().to_string());
                let script = download_notification_script(
                    &event.status,
                    event.path.as_deref(),
                    event.success,
                );
                if let Err(error) = shell_webview.evaluate_script(&script) {
                    eprintln!("could not show download notification: {error:#}");
                }
                download_history.record(event);
                persist_download_history(&download_history);
                sync_downloads(&shell_webview, &download_history);
                sync_download_manager_window(&download_manager_window, &download_history);
                if let Some(last_dir) = last_download_dir {
                    let mut should_save_settings = false;
                    if let Ok(mut state) = app_state.lock()
                        && state.settings.downloads.last_dir != last_dir
                    {
                        state.settings.downloads.last_dir = last_dir.clone();
                        should_save_settings = true;
                        if let Err(error) = save_settings(&state.settings) {
                            eprintln!("could not save download last directory: {error:#}");
                        }
                    }
                    if should_save_settings {
                        sync_download_settings_last_dir(&content_webviews, &last_dir);
                    }
                }
            }
            Event::UserEvent(UserEvent::LatencyEvent(event)) => {
                let script = latency_notification_script(&event);
                if let Err(error) = shell_webview.evaluate_script(&script) {
                    eprintln!("could not show latency notification: {error:#}");
                }
            }
            Event::UserEvent(UserEvent::StartupProgress(progress)) => {
                let script = startup_progress_script(&progress);
                for webview in content_webviews.values() {
                    if let Err(error) = webview.evaluate_script(&script) {
                        eprintln!("could not update startup progress: {error:#}");
                    }
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
            Event::UserEvent(UserEvent::RuntimeFailed(error_message)) => {
                runtime_ready = false;
                let script = runtime_failed_script(&error_message);
                for webview in content_webviews.values() {
                    if let Err(error) = webview.evaluate_script(&script) {
                        eprintln!(
                            "could not notify content webview about runtime failure: {error:#}"
                        );
                    }
                }
            }
            Event::LoopDestroyed => {
                stop_runtime(&app_state);
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                window_id,
                ..
            } if window_id == main_window_id => {
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
                window_id,
                ..
            } => {
                if download_manager_window
                    .as_ref()
                    .is_some_and(|manager| manager.window.id() == window_id)
                {
                    download_manager_window = None;
                } else if window_id == main_window_id {
                    stop_runtime(&app_state);
                    *control_flow = ControlFlow::Exit;
                }
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

        waiting_page_url(site)
    } else {
        site.url().to_string()
    }
}

fn waiting_page_url(site: AiSite) -> String {
    let startup_stage_keys = StartupStage::all()
        .iter()
        .map(|stage| stage.key())
        .collect::<Vec<_>>()
        .join(",");
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
    .status { margin: 14px 0; padding: 12px; border: 1px solid #d9dde3; border-radius: 8px; background: #f7f8fa; }
    .actions { display: none; gap: 8px; margin-top: 14px; flex-wrap: wrap; }
    .actions[data-visible="true"] { display: flex; }
    button { border: 1px solid #cfd5df; border-radius: 7px; background: #fff; padding: 7px 10px; cursor: pointer; }
  </style>
</head>
<body>
  <main>
    <h1>正在启动内置代理</h1>
    <p>代理就绪后会自动打开 __SITE_TITLE__。右下角“设”可以查看订阅、节点和日志。</p>
    <div class="status" data-startup-stages="__STARTUP_STAGE_KEYS__">
      <p>当前步骤：<strong data-startup-stage>读取配置</strong></p>
      <p>已用时：<span data-startup-elapsed>0</span>s</p>
      <p data-startup-message></p>
    </div>
    <div class="actions" data-startup-actions data-visible="false">
      <button type="button" onclick="window.location.reload()">重试代理</button>
      <button type="button" onclick="window.location.href='__SITE_URL__'">跳过代理打开</button>
      <button type="button" onclick="window.location.href='__SITE_URL__'">继续打开</button>
    </div>
  </main>
  <script>
    const startupStartedAt = Date.now();
    setInterval(() => {
      const elapsed = document.querySelector('[data-startup-elapsed]');
      if (elapsed) elapsed.textContent = String(Math.floor((Date.now() - startupStartedAt) / 1000));
    }, 1000);
    setTimeout(() => {
      document.querySelector('[data-startup-actions]')?.setAttribute('data-visible', 'true');
    }, 15000);
  </script>
</body>
</html>"#
        .replace("__SITE_TITLE__", site.title())
        .replace("__SITE_URL__", site.url())
        .replace("__STARTUP_STAGE_KEYS__", &startup_stage_keys);
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

fn runtime_failed_script(error_message: &str) -> String {
    let payload = json!({
        "title": "内置代理启动失败",
        "message": error_message,
        "hint": "请确认程序包里存在 resources/clash/mihomo，或重新解压完整安装包/便携包后启动。",
    });
    format!(
        r#"
(() => {{
  const payload = {payload};
  const isWaitingPage = window.location.href.startsWith('data:text/html') || window.location.href === 'about:blank';
  if (!isWaitingPage) {{
    window.__chatgptClientReceive && window.__chatgptClientReceive({{
      id: null,
      ok: false,
      error: payload.message
    }});
    return;
  }}
  document.body.innerHTML = '';
  document.body.style.cssText = 'margin:0;min-height:100vh;display:grid;place-items:center;background:#fff;color:#111;font:16px/1.5 system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;';
  const main = document.createElement('main');
  main.style.cssText = 'width:min(680px,calc(100vw - 40px));';
  const title = document.createElement('h1');
  title.textContent = payload.title;
  title.style.cssText = 'margin:0 0 10px;font-size:24px;';
  const message = document.createElement('pre');
  message.textContent = payload.message;
  message.style.cssText = 'white-space:pre-wrap;margin:12px 0;padding:12px;border:1px solid #ddd;border-radius:8px;background:#f7f7f8;color:#333;font:13px/1.45 ui-monospace,SFMono-Regular,Consolas,monospace;';
  const hint = document.createElement('p');
  hint.textContent = payload.hint;
  hint.style.cssText = 'margin:6px 0;color:#555;';
  main.append(title, hint, message);
  document.body.appendChild(main);
}})();
"#
    )
}

fn startup_progress_script(progress: &StartupProgress) -> String {
    let payload = json!({
        "stage": progress.stage.key(),
        "label": progress.stage.label(),
        "elapsed_secs": progress.elapsed_secs,
        "message": progress.message,
    });
    format!(
        r#"
(() => {{
  const payload = {payload};
  const stage = document.querySelector('[data-startup-stage]');
  const elapsed = document.querySelector('[data-startup-elapsed]');
  const message = document.querySelector('[data-startup-message]');
  if (stage) stage.textContent = payload.label || payload.stage || '';
  if (elapsed) elapsed.textContent = String(payload.elapsed_secs || 0);
  if (message) message.textContent = payload.message || '';
  if (payload.stage === 'failed') {{
    document.querySelector('[data-startup-actions]')?.setAttribute('data-visible', 'true');
  }}
}})();
"#
    )
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

fn new_download_ipc_token(site: AiSite) -> String {
    let sequence = DOWNLOAD_IPC_TOKEN_SEQ.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{timestamp:x}-{sequence:x}", site.key())
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
    download_ipc_token: &str,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<WebView> {
    let target = IpcTarget::Site(site);
    let download_started_proxy = event_proxy.clone();
    let download_completed_proxy = event_proxy.clone();
    let native_download_settings = settings.downloads.clone();
    let builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(site_initial_url(settings, site, runtime_ready))
        .with_bounds(bounds)
        .with_visible(visible)
        .with_initialization_script(download_interceptor_script(download_ipc_token))
        .with_initialization_script(settings_button_script(settings_json))
        .with_ipc_handler(move |request| {
            let _ = event_proxy.send_event(UserEvent::Ipc {
                target,
                body: request.body().to_string(),
            });
        })
        .with_download_started_handler(move |url, path| {
            let destination =
                download_destination_for_with_settings(path, &native_download_settings);
            eprintln!("download started: {url} -> {}", destination.display());
            *path = destination;
            let _ = download_started_proxy.send_event(UserEvent::DownloadEvent(DownloadEvent {
                kind: DownloadEventKind::Started,
                status: "下载已开始".to_string(),
                path: Some(path.clone()),
                url: Some(url),
                bytes: None,
                success: true,
            }));
            true
        })
        .with_download_completed_handler(move |url, path, success| {
            if success {
                if let Some(path) = &path {
                    eprintln!("download completed: {url} -> {}", path.display());
                } else {
                    eprintln!("download completed: {url}");
                }
            } else {
                eprintln!("download failed: {url}");
            }
            let _ = download_completed_proxy.send_event(UserEvent::DownloadEvent(DownloadEvent {
                kind: if success {
                    DownloadEventKind::Completed
                } else {
                    DownloadEventKind::Failed
                },
                status: if success {
                    "下载完成".to_string()
                } else {
                    "下载失败".to_string()
                },
                path,
                url: Some(url),
                bytes: None,
                success,
            }));
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
    download_ipc_tokens: &mut HashMap<AiSite, String>,
    event_proxy: EventLoopProxy<UserEvent>,
) -> Result<()> {
    if let std::collections::hash_map::Entry::Vacant(entry) = content_webviews.entry(site) {
        let (width, height) = logical_window_size(window);
        let download_ipc_token = new_download_ipc_token(site);
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
            &download_ipc_token,
            event_proxy,
        )?;
        download_ipc_tokens.insert(site, download_ipc_token);
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

fn sync_downloads(shell_webview: &WebView, history: &DownloadHistory) {
    let payload = history.payload();
    let script =
        format!("window.__aiClientSyncDownloads && window.__aiClientSyncDownloads({payload});");
    if let Err(error) = shell_webview.evaluate_script(&script) {
        eprintln!("could not sync download state: {error:#}");
    }
}

fn download_manager_html(history: &DownloadHistory) -> String {
    let initial_payload = history.payload();
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>下载管理</title>
  <style>
    :root {{ --surface: #fff; --surface-muted: #f5f6f8; --line: #d9dde3; --line-strong: #a8b0bd; --text: #15181d; --muted: #687080; --ok: #0b7a3b; --bad: #9b1c1c; --blue: #2457b0; }}
    * {{ box-sizing: border-box; }}
    html, body {{ width: 100%; height: 100%; margin: 0; overflow: hidden; background: var(--surface); color: var(--text); font: 13px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
    button, input {{ font: inherit; }}
    body {{ display: grid; grid-template-rows: auto auto minmax(0, 1fr); }}
    header {{ display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 16px 18px 12px; border-bottom: 1px solid var(--line); }}
    h1 {{ margin: 0; font-size: 18px; font-weight: 700; letter-spacing: 0; }}
    .hint {{ color: var(--muted); margin-top: 3px; }}
    .actions {{ display: flex; align-items: center; gap: 8px; flex-wrap: wrap; justify-content: flex-end; }}
    .toolbar {{ display: grid; grid-template-columns: minmax(180px, 420px) auto; gap: 12px; align-items: center; padding: 12px 18px; border-bottom: 1px solid var(--line); background: var(--surface-muted); }}
    .search {{ width: 100%; height: 34px; border: 1px solid var(--line); border-radius: 7px; padding: 0 10px; background: #fff; color: var(--text); outline: none; }}
    .search:focus {{ border-color: var(--line-strong); }}
    .summary {{ justify-self: end; color: var(--muted); white-space: nowrap; }}
    .btn {{ min-height: 32px; border: 1px solid var(--line); border-radius: 7px; background: #fff; color: var(--text); padding: 0 10px; cursor: pointer; }}
    .btn:hover {{ border-color: var(--line-strong); background: #eef1f5; }}
    .btn.primary {{ background: #15181d; color: #fff; border-color: #15181d; }}
    .content {{ min-height: 0; overflow: auto; padding: 12px 18px 18px; }}
    .empty {{ display: none; place-items: center; min-height: 220px; color: var(--muted); border: 1px dashed var(--line); border-radius: 8px; background: #fafbfc; }}
    .empty[data-visible="true"] {{ display: grid; }}
    .list {{ display: grid; gap: 10px; }}
    .download-row {{ display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 12px; align-items: center; border: 1px solid var(--line); border-radius: 8px; padding: 12px; background: #fff; }}
    .download-row:hover {{ border-color: var(--line-strong); }}
    .name-row {{ display: flex; align-items: center; gap: 8px; min-width: 0; }}
    .name {{ overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 650; }}
    .status {{ flex: 0 0 auto; border: 1px solid var(--line); border-radius: 999px; padding: 1px 8px; color: var(--muted); font-size: 12px; }}
    .status[data-status="completed"] {{ border-color: #b8dbc7; color: var(--ok); }}
    .status[data-status="failed"], .status[data-status="cancelled"] {{ border-color: #f0b7b7; color: var(--bad); }}
    .status[data-status="started"] {{ border-color: #c9d9fb; color: var(--blue); }}
    .meta, .path {{ margin-top: 4px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
    .path {{ font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; }}
    .row-actions {{ display: flex; align-items: center; gap: 6px; }}
    .row-actions .btn {{ min-height: 30px; padding-inline: 9px; }}
    @media (max-width: 760px) {{
      header, .toolbar {{ display: flex; align-items: stretch; flex-direction: column; }}
      .actions, .summary {{ justify-content: flex-start; justify-self: start; }}
      .download-row {{ grid-template-columns: minmax(0, 1fr); }}
      .row-actions {{ flex-wrap: wrap; }}
    }}
  </style>
</head>
<body>
  <header>
    <div>
      <h1>下载管理</h1>
      <div class="hint">查看下载记录、打开文件位置或调整保存路径。</div>
    </div>
    <div class="actions">
      <button class="btn" type="button" data-action="openDownloadSettings">保存路径设置</button>
      <button class="btn" type="button" data-action="clearCompletedDownloads">清除已完成</button>
      <button class="btn primary" type="button" data-action="closeDownloadManager">关闭</button>
    </div>
  </header>
  <section class="toolbar" aria-label="下载筛选">
    <input class="search" type="search" placeholder="搜索文件名、路径或来源" data-download-filter>
    <div class="summary" data-download-summary>0 个下载记录</div>
  </section>
  <main class="content">
    <div class="empty" data-download-empty data-visible="false">暂无下载记录</div>
    <div class="list" data-download-list></div>
  </main>
  <script>
    let downloads = [];
    let requestSeq = 0;
    const filterInput = document.querySelector('[data-download-filter]');
    const list = document.querySelector('[data-download-list]');
    const empty = document.querySelector('[data-download-empty]');
    const summary = document.querySelector('[data-download-summary]');

    function sendCommand(type, payload = {{}}) {{
      if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
      window.ipc.postMessage(JSON.stringify({{ id: String(++requestSeq), type, payload }}));
    }}
    function escapeHtml(value) {{
      return String(value ?? '').replace(/[&<>"']/g, ch => ({{ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }}[ch]));
    }}
    function statusLabel(status) {{
      switch (status) {{
        case 'started': return '下载中';
        case 'completed': return '已完成';
        case 'failed': return '失败';
        case 'diagnostic': return '诊断';
        case 'cancelled': return '已取消';
        case 'missing': return '文件缺失';
        default: return status || '-';
      }}
    }}
    function formatBytes(bytes) {{
      if (!Number.isFinite(bytes) || bytes <= 0) return '';
      const units = ['B', 'KB', 'MB', 'GB'];
      let value = bytes;
      let unit = 0;
      while (value >= 1024 && unit < units.length - 1) {{ value /= 1024; unit += 1; }}
      return `${{value.toFixed(unit === 0 ? 0 : 1)}} ${{units[unit]}}`;
    }}
    function formatTime(timestamp) {{
      const value = Number(timestamp);
      if (!Number.isFinite(value) || value <= 0) return '';
      try {{ return new Date(value).toLocaleString(); }} catch (_) {{ return ''; }}
    }}
    function currentFilter() {{
      return (filterInput?.value || '').trim().toLowerCase();
    }}
    function filteredDownloads() {{
      const query = currentFilter();
      if (!query) return downloads;
      return downloads.filter(item => [item.filename, item.path, item.url, item.message, item.status]
        .some(value => String(value || '').toLowerCase().includes(query)));
    }}
    function render() {{
      const items = filteredDownloads();
      const completed = downloads.filter(item => item.status === 'completed').length;
      const failed = downloads.filter(item => item.status === 'failed').length;
      summary.textContent = `${{downloads.length}} 个记录，${{completed}} 个完成，${{failed}} 个失败`;
      empty.dataset.visible = String(items.length === 0);
      list.innerHTML = items.map(item => {{
        const path = item.path || '';
        const meta = [formatTime(item.timestamp_ms), formatBytes(item.bytes), item.message || item.url || ''].filter(Boolean).join(' · ');
        const openDisabled = path ? '' : ' disabled';
        return `<article class="download-row" data-download-id="${{item.id}}">
          <div class="download-main">
            <div class="name-row">
              <div class="name" title="${{escapeHtml(item.filename || 'download')}}">${{escapeHtml(item.filename || 'download')}}</div>
              <span class="status" data-status="${{escapeHtml(item.status || 'missing')}}">${{escapeHtml(statusLabel(item.status))}}</span>
            </div>
            <div class="meta" title="${{escapeHtml(meta)}}">${{escapeHtml(meta || '-')}}</div>
            <div class="path" title="${{escapeHtml(path || item.url || '')}}">${{escapeHtml(path || item.url || '无本地路径')}}</div>
          </div>
          <div class="row-actions">
            <button class="btn" type="button" data-action="openDownloadPath" data-path="${{escapeHtml(path)}}"${{openDisabled}}>打开文件</button>
            <button class="btn" type="button" data-action="openDownloadFolder" data-path="${{escapeHtml(path)}}"${{openDisabled}}>打开目录</button>
            <button class="btn" type="button" data-action="deleteDownloadRecord" data-id="${{item.id}}">删除记录</button>
          </div>
        </article>`;
      }}).join('');
    }}
    window.__aiClientSyncDownloads = (payload = {{}}) => {{
      downloads = Array.isArray(payload.downloads) ? payload.downloads : [];
      render();
    }};
    filterInput?.addEventListener('input', render);
    document.addEventListener('click', event => {{
      const button = event.target.closest('[data-action]');
      if (!button) return;
      const action = button.dataset.action;
      if (action === 'openDownloadPath') sendCommand(action, {{ path: button.dataset.path || '' }});
      else if (action === 'openDownloadFolder') sendCommand(action, {{ path: button.dataset.path || '' }});
      else if (action === 'deleteDownloadRecord') sendCommand(action, {{ id: Number(button.dataset.id || 0) }});
      else sendCommand(action);
    }});
    window.addEventListener('keydown', event => {{
      if (event.key === 'Escape') sendCommand('closeDownloadManager');
    }});
    window.__aiClientSyncDownloads({initial_payload});
  </script>
</body>
</html>"#
    )
}

fn open_download_manager_window(
    download_manager_window: &mut Option<DownloadManagerWindow>,
    event_loop: &EventLoopWindowTarget<UserEvent>,
    event_proxy: EventLoopProxy<UserEvent>,
    history: &DownloadHistory,
) -> Result<()> {
    if let Some(manager) = download_manager_window.as_ref() {
        manager.window.set_visible(true);
        manager.window.set_focus();
        sync_download_manager_window(download_manager_window, history);
        return Ok(());
    }

    let window = WindowBuilder::new()
        .with_title("下载管理")
        .with_inner_size(LogicalSize::new(980.0, 720.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .build(event_loop)
        .context("could not create download manager window")?;
    let builder = WebViewBuilder::new()
        .with_html(download_manager_html(history))
        .with_ipc_handler(move |request| {
            let _ = event_proxy.send_event(UserEvent::Ipc {
                target: IpcTarget::DownloadManager,
                body: request.body().to_string(),
            });
        });

    #[cfg(windows)]
    let builder = apply_windows_diagnostics(builder, None);

    let webview = builder
        .build(&window)
        .context("could not build download manager webview")?;
    window.set_focus();

    *download_manager_window = Some(DownloadManagerWindow { window, webview });
    sync_download_manager_window(download_manager_window, history);

    Ok(())
}

fn sync_download_manager_window(
    download_manager_window: &Option<DownloadManagerWindow>,
    history: &DownloadHistory,
) {
    let Some(manager) = download_manager_window.as_ref() else {
        return;
    };
    let payload = history.payload();
    let script =
        format!("window.__aiClientSyncDownloads && window.__aiClientSyncDownloads({payload});");
    if let Err(error) = manager.webview.evaluate_script(&script) {
        eprintln!("could not sync download manager state: {error:#}");
    }
}

fn sync_download_settings_last_dir(content_webviews: &HashMap<AiSite, WebView>, last_dir: &str) {
    let payload = json!({ "last_dir": last_dir });
    let script = format!(
        "window.__chatgptClientUpdateDownloadSettings && window.__chatgptClientUpdateDownloadSettings({payload});"
    );

    for webview in content_webviews.values() {
        if let Err(error) = webview.evaluate_script(&script) {
            eprintln!("could not sync download settings: {error:#}");
        }
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
    .latency-pill { flex: 0 0 auto; min-width: 104px; justify-content: center; overflow: visible; }
    .pill[data-health="good"] { border-color: #b8dbc7; color: #0b7a3b; }
    .pill[data-health="bad"] { border-color: #f0b7b7; color: #9b1c1c; }
    .pill strong { color: var(--text); font-weight: 650; }
    .toolbar-divider { width: 1px; height: 24px; background: var(--line); margin: 0 4px; }
    .icon-btn { display: grid; place-items: center; width: 34px; height: 34px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); color: var(--text); cursor: pointer; }
    .icon-btn:hover { border-color: var(--line-strong); background: var(--surface-muted); }
    .icon-btn svg { width: 17px; height: 17px; stroke: currentColor; stroke-width: 2; fill: none; stroke-linecap: round; stroke-linejoin: round; }
    .toast-region { position: fixed; right: 12px; top: 8px; z-index: 30; display: grid; gap: 8px; width: min(360px, calc(100vw - 24px)); pointer-events: none; }
    .toast { border: 1px solid var(--line); border-radius: 8px; background: rgba(255,255,255,.98); box-shadow: 0 10px 28px rgba(22,31,45,.16); padding: 7px 10px; color: var(--text); animation: toast-in .16s ease-out; }
    .toast[data-success="false"] { border-color: #f0b7b7; }
    .toast-title { font-weight: 650; }
    .toast-path { display: block; width: 100%; margin: 3px 0 0; border: 0; background: transparent; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; text-align: left; cursor: pointer; padding: 0; pointer-events: auto; }
    .toast-path:hover { color: var(--text); text-decoration: underline; }
    @keyframes toast-in { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }
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
        <span class="pill latency-pill" data-latency-label data-health="unknown">延时 --</span>
        <span class="pill"><strong>已登录</strong></span>
        <span class="pill"><strong>代理可用</strong></span>
        <span class="pill"><strong>页面常驻</strong></span>
      </div>
      <span class="toolbar-divider" aria-hidden="true"></span>
      <button class="icon-btn" type="button" title="后退" aria-label="后退" data-action="navBack"><svg viewBox="0 0 24 24"><path d="M15 18l-6-6 6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="前进" aria-label="前进" data-action="navForward"><svg viewBox="0 0 24 24"><path d="M9 18l6-6-6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="刷新" aria-label="刷新" data-action="reloadActive"><svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-2.64-6.36"></path><path d="M21 4v6h-6"></path></svg></button>
      <button class="icon-btn" type="button" title="导出 Markdown" aria-label="导出 Markdown" data-action="exportMarkdown"><svg viewBox="0 0 24 24"><path d="M4 4h16v16H4z"></path><path d="M7 16V8l3 4 3-4v8"></path><path d="M17 8v8"></path><path d="M15 14l2 2 2-2"></path></svg></button>
      <button class="icon-btn" type="button" title="导出 PDF" aria-label="导出 PDF" data-action="exportPdf"><svg viewBox="0 0 24 24"><path d="M6 3h9l3 3v15H6z"></path><path d="M14 3v4h4"></path><path d="M8 16h8"></path><path d="M8 12h8"></path></svg></button>
      <button class="icon-btn" type="button" title="下载管理" aria-label="下载管理" data-action="openDownloadManager"><svg viewBox="0 0 24 24"><path d="M12 3v10"></path><path d="M8 9l4 4 4-4"></path><path d="M4 17h16"></path><path d="M6 21h12"></path></svg></button>
      <button class="icon-btn" type="button" title="清理内存" aria-label="清理内存" data-action="optimizeMemory"><svg viewBox="0 0 24 24"><path d="M3 17h18"></path><path d="M5 17l2-10h10l2 10"></path><path d="M8 21h8"></path><path d="M9 7V3h6v4"></path></svg></button>
    </div>
  </header>
  <div class="toast-region" aria-live="polite" aria-atomic="true"></div>
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
    function measureActiveLatency() {
      const label = document.querySelector('[data-latency-label]');
      if (label) {
        label.textContent = '延时 ...';
        label.dataset.health = 'unknown';
      }
      sendCommand('measureLatency', { site: activeSite });
    }
    window.__aiClientSyncTabs = (active, loaded) => {
      started.clear();
      (Array.isArray(loaded) ? loaded : ['chatgpt']).forEach(site => started.add(site));
      started.add('chatgpt');
      activeSite = active || 'chatgpt';
      setActive(activeSite);
      measureActiveLatency();
    };
    window.__aiClientUpdateLatency = ({ site, delay_ms, success } = {}) => {
      if (site && site !== activeSite) return;
      const label = document.querySelector('[data-latency-label]');
      if (!label) return;
      if (success && Number.isFinite(delay_ms)) {
        label.textContent = `延时 ${delay_ms}ms`;
        label.dataset.health = delay_ms < 3000 ? 'good' : 'bad';
      } else {
        label.textContent = '延时失败';
        label.dataset.health = 'bad';
      }
    };
    window.__aiClientNotifyDownload = ({ status, path, success } = {}) => {
      const region = document.querySelector('.toast-region');
      if (!region) return;
      const toast = document.createElement('div');
      toast.className = 'toast';
      toast.dataset.success = String(success !== false);
      const title = document.createElement('div');
      title.className = 'toast-title';
      title.textContent = status || '下载状态';
      toast.appendChild(title);
      if (path) {
        const detail = document.createElement('button');
        detail.type = 'button';
        detail.className = 'toast-path';
        detail.dataset.action = 'openDownloadPath';
        detail.dataset.path = path;
        detail.title = path;
        detail.textContent = path;
        detail.addEventListener('click', () => sendCommand('openDownloadPath', { path }));
        toast.appendChild(detail);
      }
      region.appendChild(toast);
      while (region.children.length > 3) region.firstElementChild.remove();
      setTimeout(() => toast.remove(), 5200);
    };
    document.querySelectorAll('.tab').forEach(tab => {
      tab.addEventListener('click', event => {
        if (event.target.closest('.tab-close')) return;
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
      });
      tab.addEventListener('keydown', event => {
        if (!['Enter', ' '].includes(event.key)) return;
        event.preventDefault();
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
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
        else if (button.dataset.action === 'openDownloadManager') sendCommand('openDownloadManager');
        else if (button.dataset.action === 'exportMarkdown') sendCommand('exportConversation', { format: 'markdown' });
        else if (button.dataset.action === 'exportPdf') sendCommand('exportConversation', { format: 'pdf' });
        else sendCommand(button.dataset.action);
      });
    });
    document.addEventListener('keydown', event => {
      if (event.altKey && ['1', '2', '3', '4'].includes(event.key)) {
        const site = ['chatgpt', 'gemini', 'notebooklm', 'aistudio'][Number(event.key) - 1];
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
      }
    });
    setTimeout(measureActiveLatency, 5000);
    setInterval(measureActiveLatency, 60000);
  </script>
</body>
</html>"#;

    html.replace("__TABS__", &tabs)
        .replace("__ACTIVE_SITE__", active_site.key())
}

#[allow(dead_code)]
fn top_shell_html_legacy(active_site: AiSite) -> String {
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
    .latency-pill { flex: 0 0 auto; min-width: 104px; justify-content: center; overflow: visible; }
    .pill[data-health="good"] { border-color: #b8dbc7; color: #0b7a3b; }
    .pill[data-health="bad"] { border-color: #f0b7b7; color: #9b1c1c; }
    .pill strong { color: var(--text); font-weight: 650; }
    .toolbar-divider { width: 1px; height: 24px; background: var(--line); margin: 0 4px; }
    .icon-btn { display: grid; place-items: center; width: 34px; height: 34px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); color: var(--text); cursor: pointer; }
    .icon-btn:hover { border-color: var(--line-strong); background: var(--surface-muted); }
    .icon-btn svg { width: 17px; height: 17px; stroke: currentColor; stroke-width: 2; fill: none; stroke-linecap: round; stroke-linejoin: round; }
    .toast-region { position: fixed; right: 12px; top: 8px; z-index: 30; display: grid; gap: 8px; width: min(360px, calc(100vw - 24px)); pointer-events: none; }
    .toast { border: 1px solid var(--line); border-radius: 8px; background: rgba(255,255,255,.98); box-shadow: 0 10px 28px rgba(22,31,45,.16); padding: 7px 10px; color: var(--text); animation: toast-in .16s ease-out; }
    .toast[data-success="false"] { border-color: #f0b7b7; }
    .toast-title { font-weight: 650; }
    .toast-path { display: block; width: 100%; margin: 3px 0 0; border: 0; background: transparent; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; text-align: left; cursor: pointer; padding: 0; pointer-events: auto; }
    .toast-path:hover { color: var(--text); text-decoration: underline; }
    .download-overlay { position: fixed; inset: 0; z-index: 40; display: none; align-items: center; justify-content: center; padding: 16px; background: rgba(18, 24, 34, .34); backdrop-filter: blur(3px); }
    .download-overlay[data-open="true"] { display: flex; }
    .download-panel { width: min(980px, calc(100vw - 32px)); height: min(82vh, 760px); border: 1px solid var(--line); border-radius: 8px; background: rgba(255,255,255,.99); box-shadow: 0 24px 72px rgba(22,31,45,.28); overflow: hidden; display: grid; grid-template-rows: auto minmax(0, 1fr) auto; }
    .download-head { display: grid; grid-template-columns: minmax(0, 1fr) minmax(220px, 320px) auto; align-items: center; gap: 10px; padding: 12px 14px; border-bottom: 1px solid var(--line); background: rgba(255,255,255,.98); }
    .download-head-main { display: grid; gap: 2px; min-width: 0; }
    .download-title { font-weight: 650; }
    .download-summary { color: var(--muted); font-size: 12px; }
    .download-search { width: 100%; min-height: 32px; border: 1px solid var(--line); border-radius: 8px; background: var(--surface); padding: 0 10px; color: var(--text); }
    .download-search:focus { outline: 2px solid rgba(27, 111, 237, .18); outline-offset: 1px; border-color: #9db6dd; }
    .download-head-actions { display: flex; align-items: center; gap: 6px; justify-content: flex-end; min-width: 0; overflow: auto; scrollbar-width: none; }
    .download-head-actions::-webkit-scrollbar { display: none; }
    .download-body { overflow: auto; padding: 0; display: block; background: #fff; }
    .download-empty { color: var(--muted); padding: 28px 12px; text-align: center; }
    .download-item { display: grid; grid-template-columns: 42px minmax(0, 1fr) auto; gap: 12px; align-items: center; min-height: 84px; border-bottom: 1px solid #edf0f3; padding: 10px 14px; background: var(--surface); }
    .download-item:hover { background: #fafbfc; }
    .download-row-icon { display: grid; place-items: center; width: 34px; height: 42px; color: #6a7280; }
    .download-row-icon svg { width: 32px; height: 32px; stroke: currentColor; stroke-width: 1.7; fill: none; stroke-linecap: round; stroke-linejoin: round; }
    .download-main { min-width: 0; display: grid; gap: 4px; }
    .download-name-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
    .download-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
    .download-status { flex: 0 0 auto; display: inline-flex; align-items: center; min-height: 22px; border-radius: 999px; border: 1px solid var(--line); padding: 0 8px; color: var(--muted); font-size: 12px; }
    .download-status[data-status="completed"] { border-color: #b8dbc7; color: #0b7a3b; }
    .download-status[data-status="failed"] { border-color: #f0b7b7; color: #9b1c1c; }
    .download-status[data-status="started"] { border-color: #c9d9fb; color: #2457b0; }
    .download-status[data-status="diagnostic"], .download-status[data-status="missing"] { border-color: #dde3ea; color: #6a7280; }
    .download-meta, .download-path { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .download-path { cursor: default; }
    .download-actions { display: flex; align-items: center; gap: 8px; justify-content: flex-end; }
    .download-link, .text-btn { border: 1px solid var(--line); border-radius: 7px; background: var(--surface); color: var(--text); cursor: pointer; padding: 5px 8px; }
    .download-icon-action { display: grid; place-items: center; width: 34px; height: 34px; border: 0; border-radius: 7px; background: transparent; color: var(--muted); cursor: pointer; }
    .download-icon-action:hover { background: var(--surface-muted); color: var(--text); }
    .download-icon-action svg { width: 22px; height: 22px; stroke: currentColor; stroke-width: 1.8; fill: none; stroke-linecap: round; stroke-linejoin: round; }
    .download-link:hover, .text-btn:hover { background: var(--surface-muted); }
    .download-foot { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 10px 12px; border-top: 1px solid var(--line); background: rgba(255,255,255,.98); }
    @keyframes toast-in { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }
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
        <span class="pill latency-pill" data-latency-label data-health="unknown">延时 --</span>
        <span class="pill"><strong>已登录</strong></span>
        <span class="pill"><strong>代理可用</strong></span>
        <span class="pill"><strong>页面常驻</strong></span>
      </div>
      <span class="toolbar-divider" aria-hidden="true"></span>
      <button class="icon-btn" type="button" title="后退" aria-label="后退" data-action="navBack"><svg viewBox="0 0 24 24"><path d="M15 18l-6-6 6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="前进" aria-label="前进" data-action="navForward"><svg viewBox="0 0 24 24"><path d="M9 18l6-6-6-6"></path></svg></button>
      <button class="icon-btn" type="button" title="刷新" aria-label="刷新" data-action="reloadActive"><svg viewBox="0 0 24 24"><path d="M21 12a9 9 0 1 1-2.64-6.36"></path><path d="M21 4v6h-6"></path></svg></button>
      <button class="icon-btn" type="button" title="导出 Markdown" aria-label="导出 Markdown" data-action="exportMarkdown"><svg viewBox="0 0 24 24"><path d="M4 4h16v16H4z"></path><path d="M7 16V8l3 4 3-4v8"></path><path d="M17 8v8"></path><path d="M15 14l2 2 2-2"></path></svg></button>
      <button class="icon-btn" type="button" title="导出 PDF" aria-label="导出 PDF" data-action="exportPdf"><svg viewBox="0 0 24 24"><path d="M6 3h9l3 3v15H6z"></path><path d="M14 3v4h4"></path><path d="M8 16h8"></path><path d="M8 12h8"></path></svg></button>
      <button class="icon-btn" type="button" title="下载管理" aria-label="下载管理" data-action="toggleDownloadManager"><svg viewBox="0 0 24 24"><path d="M12 3v10"></path><path d="M8 9l4 4 4-4"></path><path d="M4 17h16"></path><path d="M6 21h12"></path></svg></button>
      <button class="icon-btn" type="button" title="清理内存" aria-label="清理内存" data-action="optimizeMemory"><svg viewBox="0 0 24 24"><path d="M3 17h18"></path><path d="M5 17l2-10h10l2 10"></path><path d="M8 21h8"></path><path d="M9 7V3h6v4"></path></svg></button>
    </div>
  </header>
  <div class="toast-region" aria-live="polite" aria-atomic="true"></div>
  <section class="download-overlay" data-download-overlay data-open="false" aria-hidden="true">
    <section class="download-panel" data-download-panel role="dialog" aria-modal="true" aria-label="下载管理">
      <div class="download-head">
        <div class="download-head-main">
          <div class="download-title">下载管理</div>
          <div class="download-summary" data-download-summary>暂无下载记录</div>
        </div>
        <input class="download-search" type="search" data-download-filter placeholder="搜索文件名、路径、地址或状态" />
        <div class="download-head-actions">
          <button class="text-btn" type="button" data-action="openDownloadSettings">下载设置</button>
          <button class="text-btn" type="button" data-action="clearCompletedDownloads">清空已完成</button>
          <button class="text-btn" type="button" data-action="newDownload">新建下载</button>
          <button class="text-btn" type="button" data-action="closeDownloadManager">关闭</button>
        </div>
      </div>
      <div class="download-body" data-download-list>
        <div class="download-empty">暂无下载记录</div>
      </div>
      <div class="download-foot">
        <div class="download-summary" data-download-footer-summary>按文件名或路径筛选下载项</div>
        <div class="download-head-actions">
          <button class="text-btn" type="button" data-action="closeDownloadManager">关闭</button>
        </div>
      </div>
    </section>
  </section>
  <script>
    const started = new Set(['__ACTIVE_SITE__']);
    let activeSite = '__ACTIVE_SITE__';
    let requestSeq = 0;
    const downloadOverlay = document.querySelector('[data-download-overlay]');
    const downloadPanel = document.querySelector('[data-download-panel]');
    const downloadSearch = document.querySelector('[data-download-filter]');
    const downloadSummary = document.querySelector('[data-download-summary]');
    const downloadFooterSummary = document.querySelector('[data-download-footer-summary]');
    let downloads = [];
    let downloadPanelOpen = false;
    let downloadFilter = '';
    function sendCommand(type, payload = {}) {
      if (!window.ipc || typeof window.ipc.postMessage !== 'function') return;
      window.ipc.postMessage(JSON.stringify({ id: String(++requestSeq), type, payload }));
    }
    function formatBytes(bytes) {
      if (!Number.isFinite(bytes) || bytes < 0) return '';
      if (bytes < 1024) return `${bytes} B`;
      const units = ['KB', 'MB', 'GB', 'TB'];
      let value = bytes / 1024;
      let unit = units[0];
      for (let index = 1; index < units.length && value >= 1024; index += 1) {
        value /= 1024;
        unit = units[index];
      }
      return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
    }
    function formatTimestamp(timestamp) {
      const value = Number(timestamp);
      if (!Number.isFinite(value) || value <= 0) return '';
      return new Date(value).toLocaleString('zh-CN', { hour12: false });
    }
    function downloadStatusLabel(status) {
      switch (status) {
        case 'completed': return '已完成';
        case 'failed': return '失败';
        case 'started': return '下载中';
        case 'diagnostic': return '诊断';
        case 'cancelled': return '已取消';
        case 'missing': return '缺失';
        default: return status || '-';
      }
    }
    function matchesDownloadFilter(item) {
      const filter = downloadFilter.trim().toLowerCase();
      if (!filter) return true;
      const haystack = [
        item.filename,
        item.path,
        item.url,
        item.message,
        item.status,
        item.bytes != null ? String(item.bytes) : '',
      ].join('\n').toLowerCase();
      return haystack.includes(filter);
    }
    function updateDownloadSummary(visibleCount = downloads.length) {
      const total = downloads.length;
      const summary = total
        ? (downloadFilter.trim()
          ? `已显示 ${visibleCount} / ${total} 项`
          : `共 ${total} 项`)
        : '暂无下载记录';
      if (downloadSummary) downloadSummary.textContent = summary;
      if (downloadFooterSummary) {
        downloadFooterSummary.textContent = total
          ? (downloadFilter.trim() ? `筛选后 ${visibleCount} / ${total} 项` : `共 ${total} 项`)
          : '暂无下载记录';
      }
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
    function measureActiveLatency() {
      const label = document.querySelector('[data-latency-label]');
      if (label) {
        label.textContent = '延时 ...';
        label.dataset.health = 'unknown';
      }
      sendCommand('measureLatency', { site: activeSite });
    }
    function setDownloadPanelOpen(open) {
      downloadPanelOpen = Boolean(open);
      if (downloadOverlay) {
        downloadOverlay.dataset.open = String(downloadPanelOpen);
        downloadOverlay.setAttribute('aria-hidden', String(!downloadPanelOpen));
      }
      if (downloadPanelOpen) {
        sendCommand('setDownloadManagerOpen', { open: true });
        renderDownloads();
        window.setTimeout(() => {
          downloadSearch?.focus();
          downloadSearch?.select();
        }, 0);
      } else {
        sendCommand('setDownloadManagerOpen', { open: false });
        updateDownloadSummary();
      }
    }
    function toggleDownloadPanel() {
      setDownloadPanelOpen(!downloadPanelOpen);
    }
    function clearCompletedDownloads() {
      downloads = downloads.filter(item => item.status !== 'completed');
      renderDownloads();
      sendCommand('clearCompletedDownloads');
    }
    function renderDownloads() {
      const list = document.querySelector('[data-download-list]');
      if (!list) return;
      if (!downloadPanelOpen) {
        updateDownloadSummary();
        return;
      }

      const filtered = downloads.filter(matchesDownloadFilter);
      updateDownloadSummary(filtered.length);
      list.replaceChildren();

      if (!downloads.length) {
        const empty = document.createElement('div');
        empty.className = 'download-empty';
        empty.textContent = '暂无下载记录';
        list.appendChild(empty);
        return;
      }

      if (!filtered.length) {
        const empty = document.createElement('div');
        empty.className = 'download-empty';
        empty.textContent = '没有匹配的下载记录';
        list.appendChild(empty);
        return;
      }

      filtered.forEach(item => {
        const row = document.createElement('div');
        row.className = 'download-item';
        const icon = document.createElement('div');
        icon.className = 'download-row-icon';
        icon.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3h9l3 3v15H6z"></path><path d="M14 3v4h4"></path><path d="M8 15h8"></path><path d="M8 11h8"></path></svg>';
        const main = document.createElement('div');
        main.className = 'download-main';
        const nameRow = document.createElement('div');
        nameRow.className = 'download-name-row';
        const name = document.createElement('div');
        name.className = 'download-name';
        name.textContent = item.filename || 'download';
        const status = document.createElement('span');
        status.className = 'download-status';
        status.dataset.status = item.status || 'missing';
        status.textContent = downloadStatusLabel(item.status);
        nameRow.append(name, status);
        const meta = document.createElement('div');
        meta.className = 'download-meta';
        const details = [formatBytes(item.bytes), formatTimestamp(item.timestamp_ms)].filter(Boolean);
        meta.textContent = details.length ? details.join(' · ') : (item.message || item.status || '-');
        const path = document.createElement('div');
        path.className = 'download-path';
        path.title = item.path || item.url || '';
        path.textContent = item.path || item.url || item.message || '';
        main.append(nameRow, meta, path);
        const actions = document.createElement('div');
        actions.className = 'download-actions';
        if (item.path) {
          const open = document.createElement('button');
          open.className = 'download-icon-action';
          open.type = 'button';
          open.title = '打开';
          open.setAttribute('aria-label', '打开');
          open.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3h9l3 3v15H6z"></path><path d="M14 3v4h4"></path></svg>';
          open.addEventListener('click', () => sendCommand('openDownloadPath', { path: item.path }));
          const folder = document.createElement('button');
          folder.className = 'download-icon-action';
          folder.type = 'button';
          folder.title = '打开所在文件夹';
          folder.setAttribute('aria-label', '打开所在文件夹');
          folder.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6h7l2 2h9v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"></path></svg>';
          folder.addEventListener('click', () => sendCommand('openDownloadFolder', { path: item.path }));
          actions.append(open, folder);
        }
        const remove = document.createElement('button');
        remove.className = 'download-icon-action';
        remove.type = 'button';
        remove.title = '删除记录';
        remove.setAttribute('aria-label', '删除记录');
        remove.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M18 6L6 18"></path><path d="M6 6l12 12"></path></svg>';
        remove.addEventListener('click', () => {
          downloads = downloads.filter(record => record.id !== item.id);
          renderDownloads();
          sendCommand('deleteDownloadRecord', { id: item.id });
        });
        actions.append(remove);
        row.append(icon, main, actions);
        list.appendChild(row);
      });
    }
    window.__aiClientSyncTabs = (active, loaded) => {
      started.clear();
      (Array.isArray(loaded) ? loaded : ['chatgpt']).forEach(site => started.add(site));
      started.add('chatgpt');
      activeSite = active || 'chatgpt';
      setActive(activeSite);
      measureActiveLatency();
    };
    window.__aiClientSyncDownloads = (payload = {}) => {
      downloads = Array.isArray(payload.downloads) ? payload.downloads : [];
      if (downloadPanelOpen) {
        renderDownloads();
      } else {
        updateDownloadSummary();
      }
    };
    window.__aiClientUpdateLatency = ({ site, delay_ms, success } = {}) => {
      if (site && site !== activeSite) return;
      const label = document.querySelector('[data-latency-label]');
      if (!label) return;
      if (success && Number.isFinite(delay_ms)) {
        label.textContent = `延时 ${delay_ms}ms`;
        label.dataset.health = delay_ms < 3000 ? 'good' : 'bad';
      } else {
        label.textContent = '延时失败';
        label.dataset.health = 'bad';
      }
    };
    window.__aiClientNotifyDownload = ({ status, path, success } = {}) => {
      const region = document.querySelector('.toast-region');
      if (!region) return;
      const toast = document.createElement('div');
      toast.className = 'toast';
      toast.dataset.success = String(success !== false);
      const title = document.createElement('div');
      title.className = 'toast-title';
      title.textContent = status || '下载状态';
      toast.appendChild(title);
      if (path) {
        const detail = document.createElement('button');
        detail.type = 'button';
        detail.className = 'toast-path';
        detail.dataset.action = 'openDownloadPath';
        detail.dataset.path = path;
        detail.title = path;
        detail.textContent = path;
        detail.addEventListener('click', () => sendCommand('openDownloadPath', { path }));
        toast.appendChild(detail);
      }
      region.appendChild(toast);
      while (region.children.length > 3) region.firstElementChild.remove();
      setTimeout(() => toast.remove(), 5200);
    };
    document.querySelectorAll('.tab').forEach(tab => {
      tab.addEventListener('click', event => {
        if (event.target.closest('.tab-close')) return;
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
      });
      tab.addEventListener('keydown', event => {
        if (!['Enter', ' '].includes(event.key)) return;
        event.preventDefault();
        const site = tab.dataset.site;
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
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
        else if (button.dataset.action === 'toggleDownloadManager') toggleDownloadPanel();
        else if (button.dataset.action === 'exportMarkdown') sendCommand('exportConversation', { format: 'markdown' });
        else if (button.dataset.action === 'exportPdf') sendCommand('exportConversation', { format: 'pdf' });
        else sendCommand(button.dataset.action);
      });
    });
    downloadOverlay?.addEventListener('click', event => {
      if (event.target === downloadOverlay) setDownloadPanelOpen(false);
    });
    downloadOverlay?.addEventListener('transitionend', () => {
      if (!downloadPanelOpen) sendCommand('setDownloadManagerOpen', { open: false });
    });
    downloadPanel?.addEventListener('click', event => {
      const button = event.target.closest('[data-action]');
      if (!button) return;
      if (button.dataset.action === 'toggleDownloadManager' || button.dataset.action === 'closeDownloadManager') setDownloadPanelOpen(false);
      else if (button.dataset.action === 'clearCompletedDownloads') clearCompletedDownloads();
      else if (button.dataset.action === 'openDownloadSettings' || button.dataset.action === 'newDownload') {
        setDownloadPanelOpen(false);
        sendCommand('openDownloadSettings');
      }
    });
    downloadSearch?.addEventListener('input', () => {
      downloadFilter = downloadSearch.value || '';
      renderDownloads();
    });
    downloadSearch?.addEventListener('keydown', event => {
      if (event.key === 'Escape') setDownloadPanelOpen(false);
    });
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape' && downloadPanelOpen) {
        setDownloadPanelOpen(false);
        return;
      }
      if (event.altKey && ['1', '2', '3', '4'].includes(event.key)) {
        const site = ['chatgpt', 'gemini', 'notebooklm', 'aistudio'][Number(event.key) - 1];
        setActive(site);
        sendCommand('switchSite', { site });
        measureActiveLatency();
      }
    });
    setTimeout(measureActiveLatency, 5000);
    setInterval(measureActiveLatency, 60000);
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
        .header(CACHE_CONTROL, "no-store, no-cache, max-age=0")
        .header(PRAGMA, "no-cache")
        .header(EXPIRES, "0")
        .body(Cow::Owned(top_shell_html(active_site).into_bytes()))
        .expect("top shell response headers are valid")
}

fn download_interceptor_script(download_ipc_token: &str) -> String {
    r#"
(() => {
  if (window.__chatgptClientDownloadInterceptorInstalled) return;
  window.__chatgptClientDownloadInterceptorInstalled = true;
  const downloadIpcToken = __DOWNLOAD_IPC_TOKEN__;

  const pending = new Map();
  let requestSeq = 0;
  const previousReceive = window.__chatgptClientReceive;
  window.__chatgptClientReceive = (message) => {
    const entry = message && pending.get(message.id);
    if (entry) {
      pending.delete(message.id);
      if (message.ok) entry.resolve(message.data);
      else entry.reject(new Error(message.error || '下载失败'));
      return;
    }
    if (typeof previousReceive === 'function') previousReceive(message);
  };

  function canUseNativeIpc() {
    return window.ipc && typeof window.ipc.postMessage === 'function';
  }

  function sendDownloadCommand(type, payload, timeoutMs = 30000) {
    if (!canUseNativeIpc()) return Promise.reject(new Error('当前页面暂不能保存文件'));
    const id = `download-${++requestSeq}`;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      window.ipc.postMessage(JSON.stringify({ id, type, payload }));
      setTimeout(() => {
        if (pending.delete(id)) reject(new Error('保存文件超时'));
      }, timeoutMs);
    });
  }

  function sendSaveDownload(payload) {
    return sendDownloadCommand('saveDownload', payload);
  }

  function sendDownloadUrl(payload) {
    return sendDownloadCommand('downloadUrl', Object.assign({}, payload, { token: downloadIpcToken }), 300000);
  }

  function notifyDownloadError(message) {
    if (!canUseNativeIpc()) return;
    window.ipc.postMessage(JSON.stringify({
      id: `download-error-${++requestSeq}`,
      type: 'downloadDiagnostic',
      payload: { level: 'error', message: String(message || '下载失败') }
    }));
  }

  function filenameFromAnchor(anchor, url) {
    const explicitName = anchor && anchor.getAttribute('download');
    if (explicitName && explicitName.trim()) return explicitName.trim();
    try {
      const parsed = new URL(url, window.location.href);
      const last = parsed.pathname.split('/').filter(Boolean).pop();
      return decodeURIComponent(last || 'download');
    } catch (_) {
      return 'download';
    }
  }

  function looksLikeDownloadUrl(url, anchor) {
    if (!url.startsWith('http://') && !url.startsWith('https://')) return false;
    if (anchor && anchor.hasAttribute('download')) return true;
    try {
      const parsed = new URL(url, window.location.href);
      const last = decodeURIComponent(parsed.pathname.split('/').filter(Boolean).pop() || '');
      if (/\.(7z|apk|csv|dmg|doc|docx|gz|json|md|pdf|pkg|ppt|pptx|rar|tar|txt|xls|xlsx|xml|yaml|yml|zip)$/i.test(last)) {
        return true;
      }
      const text = String(anchor && anchor.textContent || '').trim();
      return /\.(7z|apk|csv|dmg|doc|docx|gz|json|md|pdf|pkg|ppt|pptx|rar|tar|txt|xls|xlsx|xml|yaml|yml|zip)$/i.test(text);
    } catch (_) {
      return false;
    }
  }

  function hasUserActivation() {
    return !navigator.userActivation || navigator.userActivation.isActive;
  }

  function blobToBase64(blob) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onloadend = () => {
        const value = String(reader.result || '');
        resolve(value.includes(',') ? value.split(',').pop() : value);
      };
      reader.onerror = () => reject(reader.error || new Error('读取下载内容失败'));
      reader.readAsDataURL(blob);
    });
  }

  async function saveBlobUrl(url, filename) {
    const response = await fetch(url);
    if (!response.ok) throw new Error(`读取下载内容失败：${response.status}`);
    const blob = await response.blob();
    const contentBase64 = await blobToBase64(blob);
    return sendSaveDownload({ filename, content_base64: contentBase64 });
  }

  function saveDataUrl(url, filename) {
    const comma = url.indexOf(',');
    if (comma < 0) return Promise.reject(new Error('无效的 data 下载链接'));
    const meta = url.slice(0, comma);
    const body = url.slice(comma + 1);
    const contentBase64 = /;base64/i.test(meta)
      ? body
      : btoa(unescape(encodeURIComponent(decodeURIComponent(body))));
    return sendSaveDownload({ filename, content_base64: contentBase64 });
  }

  async function saveHttpUrl(url, filename) {
    try {
      const response = await fetch(url, { credentials: 'include' });
      if (!response.ok) throw new Error(`读取下载内容失败：${response.status}`);
      const blob = await response.blob();
      const contentBase64 = await blobToBase64(blob);
      return sendSaveDownload({ filename, content_base64: contentBase64 });
    } catch (error) {
      console.warn('[AI Web Client] browser-session download failed, using native URL download', error);
      return sendDownloadUrl({ url, filename });
    }
  }

  async function saveClientSideDownload(url, filename) {
    if (url.startsWith('blob:')) return saveBlobUrl(url, filename);
    if (url.startsWith('data:')) return saveDataUrl(url, filename);
    if (url.startsWith('http://') || url.startsWith('https://')) return saveHttpUrl(url, filename);
    return null;
  }

  function handleDownloadAnchor(anchor) {
    if (!anchor) return false;
    const href = anchor.href || '';
    if (!href.startsWith('blob:') && !href.startsWith('data:') && !looksLikeDownloadUrl(href, anchor)) return false;
    if ((href.startsWith('http://') || href.startsWith('https://')) && !hasUserActivation()) return false;
    const filename = filenameFromAnchor(anchor, href);
    saveClientSideDownload(href, filename).catch(error => {
      console.warn('[AI Web Client] download fallback failed', error);
      notifyDownloadError(error && error.message ? error.message : '下载失败');
    });
    return true;
  }

  document.addEventListener('click', (event) => {
    const anchor = event.target && event.target.closest && event.target.closest('a[href]');
    if (!handleDownloadAnchor(anchor)) return;
    event.preventDefault();
    event.stopPropagation();
  }, true);

  const nativeAnchorClick = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function() {
    if (handleDownloadAnchor(this)) return;
    return nativeAnchorClick.apply(this, arguments);
  };

  const nativeOpen = window.open;
  window.open = function(url, target, features) {
    if (typeof url === 'string' && (url.startsWith('blob:') || url.startsWith('data:'))) {
      saveClientSideDownload(url, filenameFromAnchor(null, url)).catch(error => {
        console.warn('[AI Web Client] popup download fallback failed', error);
      });
      return null;
    }
    if (typeof url === 'string' && looksLikeDownloadUrl(url, null)) {
      if (!hasUserActivation()) return nativeOpen ? nativeOpen.apply(window, arguments) : null;
      saveClientSideDownload(url, filenameFromAnchor(null, url)).catch(error => {
        console.warn('[AI Web Client] popup URL download failed', error);
        notifyDownloadError(error && error.message ? error.message : '下载失败');
      });
      return null;
    }
    return nativeOpen ? nativeOpen.apply(window, arguments) : null;
  };

  window.__aiClientNativeShowSaveFilePicker = window.showSaveFilePicker;
  window.showSaveFilePicker = async function(options = {}) {
    const suggestedName = options.suggestedName || 'download';
    let chunks = [];
    let closed = false;
    async function flush() {
      if (closed) return;
      closed = true;
      const blob = new Blob(chunks);
      const contentBase64 = await blobToBase64(blob);
      chunks = [];
      return sendSaveDownload({ filename: suggestedName, content_base64: contentBase64 });
    }
    return {
      kind: 'file',
      name: suggestedName,
      async createWritable() {
        return {
          async write(chunk) {
            if (chunk == null) return;
            if (chunk instanceof Blob) chunks.push(chunk);
            else if (chunk instanceof ArrayBuffer) chunks.push(new Blob([chunk]));
            else if (ArrayBuffer.isView(chunk)) chunks.push(new Blob([chunk.buffer.slice(chunk.byteOffset, chunk.byteOffset + chunk.byteLength)]));
            else if (typeof chunk === 'object' && chunk.type === 'write' && chunk.data != null) {
              const data = chunk.data;
              if (data instanceof Blob) chunks.push(data);
              else if (data instanceof ArrayBuffer) chunks.push(new Blob([data]));
              else if (ArrayBuffer.isView(data)) chunks.push(new Blob([data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength)]));
              else chunks.push(new Blob([String(data)]));
            } else if (typeof chunk === 'object' && (chunk.type === 'seek' || chunk.type === 'truncate')) {
              return;
            } else {
              chunks.push(new Blob([String(chunk)]));
            }
          },
          async close() { return flush(); },
          async abort() { chunks = []; closed = true; }
        };
      }
    };
  };

  navigator.msSaveBlob = navigator.msSaveBlob || function(blob, filename) {
    saveClientSideDownload(URL.createObjectURL(blob), filename || 'download').catch(error => {
      notifyDownloadError(error && (error.message || error));
    });
    return true;
  };
  navigator.msSaveOrOpenBlob = navigator.msSaveOrOpenBlob || navigator.msSaveBlob;

  window.addEventListener('unhandledrejection', event => {
    const message = event.reason && (event.reason.message || event.reason);
    if (String(message || '').includes('下载') || String(message || '').includes('save')) {
      notifyDownloadError(message);
    }
  });
})();
"#
    .replace("__DOWNLOAD_IPC_TOKEN__", &json!(download_ipc_token).to_string())
}

fn export_conversation_script(site: AiSite, format: ExportFormat) -> String {
    let site_title = site.title();
    let format_key = format.key();
    format!(
        r#"
(() => {{
  const format = '{format_key}';
  const siteTitle = {site_title:?};
  let requestSeq = window.__aiClientExportSeq || 0;
  window.__aiClientExportSeq = requestSeq + 1;

  function textOf(element) {{
    return (element && element.innerText ? element.innerText : '')
      .replace(/\r/g, '')
      .replace(/[ \t]+\n/g, '\n')
      .replace(/\n{{3,}}/g, '\n\n')
      .trim();
  }}

  function roleLabel(role, index) {{
    const normalized = String(role || '').toLowerCase();
    if (normalized.includes('user') || normalized.includes('human')) return '用户';
    if (normalized.includes('assistant') || normalized.includes('model') || normalized.includes('ai')) return 'AI';
    return index % 2 === 0 ? '用户' : 'AI';
  }}

  function uniqueBlocks(selectors) {{
    const seen = new Set();
    const blocks = [];
    selectors.forEach(selector => {{
      document.querySelectorAll(selector).forEach(element => {{
        if (seen.has(element)) return;
        seen.add(element);
        const text = textOf(element);
        if (text.length < 2) return;
        const role = element.getAttribute('data-message-author-role')
          || element.getAttribute('data-testid')
          || element.getAttribute('aria-label')
          || element.className
          || '';
        blocks.push({{ role, text }});
      }});
    }});
    return blocks;
  }}

  function extractConversationMarkdown() {{
    const selectors = [
      '[data-message-author-role]',
      '[data-testid*="conversation-turn"]',
      '[data-testid*="message"]',
      'message-content',
      'article',
      'main [role="listitem"]',
      'main .conversation-container',
      'main .markdown',
      'main p'
    ];
    let blocks = uniqueBlocks(selectors);
    if (blocks.length < 2) {{
      const main = document.querySelector('main') || document.body;
      const fallback = textOf(main);
      if (fallback.length > 20) blocks = [{{ role: '页面', text: fallback }}];
    }}
    const compact = [];
    blocks.forEach((block, index) => {{
      const text = block.text;
      if (!text || compact.some(existing => existing.text === text || existing.text.includes(text))) return;
      compact.push({{ role: roleLabel(block.role, index), text }});
    }});
    if (!compact.length) throw new Error('当前页面未识别到可导出的对话内容');
    return compact.map((block, index) => `## ${{index + 1}}. ${{block.role}}\n\n${{block.text}}`).join('\n\n');
  }}

  try {{
    const markdown = extractConversationMarkdown();
    window.ipc.postMessage(JSON.stringify({{
      id: `export-${{++requestSeq}}`,
      type: 'exportConversation',
      payload: {{
        format,
        site_title: siteTitle,
        url: location.href,
        title: document.title || siteTitle,
        markdown
      }}
    }}));
  }} catch (error) {{
    window.ipc.postMessage(JSON.stringify({{
      id: `export-error-${{++requestSeq}}`,
      type: 'downloadDiagnostic',
      payload: {{ level: 'error', message: `导出失败：${{error && (error.message || error)}}` }}
    }}));
  }}
}})();
"#
    )
}

fn rendered_pdf_filename(site: AiSite) -> String {
    format!(
        "{}-page-export.pdf",
        sanitize_download_filename(site.title())
    )
}

#[cfg(windows)]
fn export_current_page_pdf(webview: &WebView, site: AiSite) -> Result<DownloadEvent> {
    let filename = rendered_pdf_filename(site);
    let path = download_destination_for(Path::new(&filename));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create export directory {}", parent.display()))?;
    }

    print_webview_to_pdf(webview, &path)?;
    let bytes = std::fs::metadata(&path).ok().map(|metadata| metadata.len());

    Ok(DownloadEvent {
        kind: DownloadEventKind::Completed,
        status: "PDF 导出完成".to_string(),
        path: Some(path),
        url: Some(site.url().to_string()),
        bytes,
        success: true,
    })
}

#[cfg(not(windows))]
fn export_current_page_pdf(_webview: &WebView, site: AiSite) -> Result<DownloadEvent> {
    let bytes = export_pdf_document(site.title(), site.url());
    let path = write_download_bytes(&rendered_pdf_filename(site), &bytes)?;

    Ok(DownloadEvent {
        kind: DownloadEventKind::Completed,
        status: "PDF 导出完成".to_string(),
        path: Some(path),
        url: Some(site.url().to_string()),
        bytes: Some(bytes.len() as u64),
        success: true,
    })
}

#[cfg(windows)]
fn print_webview_to_pdf(webview: &WebView, path: &Path) -> Result<()> {
    let environment: ICoreWebView2Environment6 = webview
        .environment()
        .cast()
        .context("WebView2 runtime does not support PDF print settings")?;
    let core: ICoreWebView2_7 = webview
        .webview()
        .cast()
        .context("WebView2 runtime does not support PrintToPdf")?;
    let settings = unsafe { environment.CreatePrintSettings() }
        .context("could not create WebView2 print settings")?;
    unsafe {
        settings
            .SetOrientation(COREWEBVIEW2_PRINT_ORIENTATION_PORTRAIT)
            .context("could not set PDF orientation")?;
        settings
            .SetShouldPrintBackgrounds(true)
            .context("could not enable PDF backgrounds")?;
        settings
            .SetShouldPrintHeaderAndFooter(false)
            .context("could not disable PDF headers")?;
        settings
            .SetScaleFactor(1.0)
            .context("could not set PDF scale")?;
    }

    let path_string = path.display().to_string();
    let path_wide = CoTaskMemPWSTR::from(path_string.as_str());
    PrintToPdfCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            core.PrintToPdf(*path_wide.as_ref().as_pcwstr(), &settings, &handler)
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(|error_code, success| {
            error_code?;
            if success {
                Ok(())
            } else {
                Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                    0x8000_4005u32 as i32,
                )))
            }
        }),
    )
    .map_err(|error| anyhow::anyhow!("WebView2 PrintToPdf failed: {error}"))
}

fn parse_latency_event(body: &str) -> Option<LatencyEvent> {
    let message = serde_json::from_str::<Value>(body).ok()?;
    if ipc_command(&message) != "latencyResult" {
        return None;
    }
    let payload = message.get("payload")?;
    let site = payload
        .get("site")
        .and_then(Value::as_str)
        .and_then(AiSite::from_key)?;
    let delay_ms = payload.get("delay_ms").and_then(Value::as_u64);
    let success = payload
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(delay_ms.is_some());
    let error = payload
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string);

    Some(LatencyEvent {
        site,
        delay_ms,
        success,
        error,
    })
}

fn latency_notification_script(event: &LatencyEvent) -> String {
    let payload = json!({
        "site": event.site.key(),
        "delay_ms": event.delay_ms,
        "success": event.success,
        "error": event.error,
    });
    format!("window.__aiClientUpdateLatency && window.__aiClientUpdateLatency({payload});")
}

fn measure_site_latency(site: AiSite, app_state: &Arc<Mutex<AppRuntimeState>>) -> LatencyEvent {
    let proxy = latency_proxy_snapshot(app_state);
    match measure_site_latency_with_proxy(site, proxy.as_ref()) {
        Ok(delay_ms) => LatencyEvent {
            site,
            delay_ms: Some(delay_ms),
            success: true,
            error: None,
        },
        Err(error) => LatencyEvent {
            site,
            delay_ms: None,
            success: false,
            error: Some(format!("{error:#}")),
        },
    }
}

fn measure_site_latency_with_proxy(site: AiSite, proxy: Option<&ProxySettings>) -> Result<u64> {
    let client = latency_http_client(proxy)?;
    let started = Instant::now();
    client
        .get(latency_url_for_site(site))
        .header("cache-control", "no-cache")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("could not reach {}", site.title()))?;

    Ok(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX))
}

fn latency_http_client(proxy: Option<&ProxySettings>) -> Result<Client> {
    let timeout = Duration::from_millis(DELAY_TIMEOUT_MS);
    let mut builder = Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .user_agent("EasyGPT/0.1 latency-check");

    if let Some(proxy_settings) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url(proxy_settings)?)?);
    }

    builder
        .build()
        .context("could not build latency check HTTP client")
}

fn latency_proxy_snapshot(app_state: &Arc<Mutex<AppRuntimeState>>) -> Option<ProxySettings> {
    let Ok(state) = app_state.lock() else {
        return None;
    };

    match state.settings.proxy.mode {
        ProxyMode::Direct => None,
        ProxyMode::System => startup_proxy(&state.settings),
        ProxyMode::InternalClash => state
            .clash_runtime
            .as_ref()
            .map(ClashRuntime::proxy_settings)
            .or_else(|| startup_proxy(&state.settings)),
    }
}

fn latency_url_for_site(site: AiSite) -> &'static str {
    match site {
        AiSite::ChatGpt => DELAY_TEST_URL,
        _ => site.url(),
    }
}

fn proxy_url(proxy_settings: &ProxySettings) -> Result<String> {
    let scheme = match proxy_settings.scheme {
        ProxyScheme::Http => "http",
        ProxyScheme::Socks5 => "socks5h",
    };
    if proxy_settings.host.trim().is_empty() || proxy_settings.port.trim().is_empty() {
        anyhow::bail!("proxy host or port is empty");
    }

    Ok(format!(
        "{scheme}://{}:{}",
        proxy_settings.host.trim(),
        proxy_settings.port.trim()
    ))
}

fn should_spawn_initial_runtime_start(mode: ProxyMode, already_spawned: bool) -> bool {
    matches!(mode, ProxyMode::InternalClash) && !already_spawned
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
        let started = Instant::now();
        let send_progress = |stage: StartupStage, message: Option<String>| {
            let _ = event_proxy.send_event(UserEvent::StartupProgress(StartupProgress {
                stage,
                elapsed_secs: started.elapsed().as_secs(),
                message,
            }));
        };
        send_progress(StartupStage::LoadSubscription, None);
        let Ok(mut state) = app_state.lock() else {
            return;
        };
        if matches!(state.settings.proxy.mode, ProxyMode::InternalClash) {
            send_progress(StartupStage::StartMihomo, None);
            if state.restart_clash_runtime() {
                send_progress(StartupStage::Ready, None);
                let _ = event_proxy.send_event(UserEvent::RuntimeReady);
            } else {
                let message = state
                    .runtime_error
                    .clone()
                    .unwrap_or_else(|| "internal Clash runtime failed to start".to_string());
                send_progress(StartupStage::Failed, Some(message.clone()));
                let _ = event_proxy.send_event(UserEvent::RuntimeFailed(message));
            }
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

fn is_save_download_request(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|message| ipc_command(&message) == "saveDownload")
}

fn is_download_url_request(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|message| ipc_command(&message) == "downloadUrl")
}

fn is_export_conversation_request(body: &str) -> bool {
    serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|message| ipc_command(&message) == "exportConversation")
}

fn parse_download_diagnostic_event(body: &str) -> Option<DownloadEvent> {
    let message = serde_json::from_str::<Value>(body).ok()?;
    if ipc_command(&message) != "downloadDiagnostic" {
        return None;
    }

    let payload = message.get("payload")?;
    let level = payload
        .get("level")
        .and_then(Value::as_str)
        .unwrap_or("info");
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("下载诊断事件");

    Some(DownloadEvent {
        kind: DownloadEventKind::Diagnostic,
        status: format!("下载诊断：{message}"),
        path: None,
        url: None,
        bytes: None,
        success: level != "error",
    })
}

fn handle_export_conversation_message_with_event(body: &str) -> (String, DownloadEvent) {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(message) => message,
        Err(error) => {
            return (
                ipc_error(None, format!("ignored invalid export IPC: {error}")),
                DownloadEvent {
                    kind: DownloadEventKind::Failed,
                    status: "导出失败".to_string(),
                    path: None,
                    url: None,
                    bytes: None,
                    success: false,
                },
            );
        }
    };
    let id = ipc_message_id(&message);
    match export_conversation_payload(&message) {
        Ok(payload) => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let bytes = payload.get("bytes").and_then(Value::as_u64);
            (
                ipc_ok(id.as_deref(), payload),
                DownloadEvent {
                    kind: DownloadEventKind::Completed,
                    status: "导出完成".to_string(),
                    path,
                    url: None,
                    bytes,
                    success: true,
                },
            )
        }
        Err(error) => (
            ipc_error(id.as_deref(), format!("{error:#}")),
            DownloadEvent {
                kind: DownloadEventKind::Failed,
                status: format!("导出失败：{error:#}"),
                path: None,
                url: None,
                bytes: None,
                success: false,
            },
        ),
    }
}

fn export_conversation_payload(message: &Value) -> Result<Value> {
    let payload = message
        .get("payload")
        .context("exportConversation payload is missing")?;
    let format = payload
        .get("format")
        .and_then(Value::as_str)
        .and_then(ExportFormat::from_key)
        .context("export format is missing or invalid")?;
    let site_title = payload
        .get("site_title")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("AI Conversation");
    let url = payload.get("url").and_then(Value::as_str).unwrap_or("");
    let markdown = payload
        .get("markdown")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("当前页面未识别到可导出的对话内容")?;
    let document = export_markdown_document(site_title, url, markdown);
    let bytes = match format {
        ExportFormat::Markdown => document.into_bytes(),
        ExportFormat::Pdf => export_pdf_document(site_title, &document),
    };
    let filename = format!(
        "{}-conversation.{}",
        sanitize_download_filename(site_title),
        format.extension()
    );
    let path = write_download_bytes(&filename, &bytes)?;

    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "format": format.key(),
    }))
}

fn export_markdown_document(site_title: &str, url: &str, markdown: &str) -> String {
    let mut output = String::new();
    output.push_str("# ");
    output.push_str(site_title.trim().if_empty("AI Conversation"));
    output.push_str("\n\n");
    if !url.trim().is_empty() {
        output.push_str("来源：<");
        output.push_str(url.trim());
        output.push_str(">\n\n");
    }
    output.push_str("---\n\n");
    output.push_str(markdown.trim());
    output.push('\n');
    output
}

trait IfEmpty {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str;
}

impl IfEmpty for str {
    fn if_empty<'a>(&'a self, fallback: &'a str) -> &'a str {
        if self.is_empty() { fallback } else { self }
    }
}

fn export_pdf_document(title: &str, markdown: &str) -> Vec<u8> {
    let lines = wrap_pdf_lines(markdown, 58);
    let pages = lines
        .chunks(48)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    let pages = if pages.is_empty() {
        vec![vec![String::from("(empty)")]]
    } else {
        pages
    };

    let mut objects = Vec::new();
    objects.push(String::from("<< /Type /Catalog /Pages 2 0 R >>"));

    let page_ids = (0..pages.len())
        .map(|index| 3 + index * 2)
        .collect::<Vec<_>>();
    let kids = page_ids
        .iter()
        .map(|id| format!("{id} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!(
        "<< /Type /Pages /Kids [{kids}] /Count {} >>",
        page_ids.len()
    ));

    let font_id = 3 + pages.len() * 2;
    let descendant_font_id = font_id + 1;
    let cid_info_id = font_id + 2;

    for (index, page_lines) in pages.iter().enumerate() {
        let page_id = 3 + index * 2;
        let content_id = page_id + 1;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {content_id} 0 R >>"
        ));
        let stream = pdf_page_stream(title, page_lines);
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            stream.len(),
            stream
        ));
    }

    objects.push(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [{descendant_font_id} 0 R] >>"
    ));
    objects.push(format!(
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo {cid_info_id} 0 R /DW 1000 /W [1 [500]] >>"
    ));
    objects.push(String::from(
        "<< /Registry (Adobe) /Ordering (GB1) /Supplement 5 >>",
    ));

    let mut pdf = String::from("%PDF-1.4\n% EasyGPT\n");
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        let object_id = index + 1;
        let _ = write!(pdf, "{object_id} 0 obj\n{object}\nendobj\n");
    }
    let xref_offset = pdf.len();
    let _ = write!(pdf, "xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        let _ = writeln!(pdf, "{offset:010} 00000 n ");
    }
    let _ = write!(
        pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objects.len() + 1,
        xref_offset
    );
    pdf.into_bytes()
}

fn pdf_page_stream(title: &str, lines: &[String]) -> String {
    let mut stream = String::from("BT\n/F1 10 Tf\n50 800 Td\n");
    let heading = format!("{} - EasyGPT Export", title);
    let _ = writeln!(stream, "<{}> Tj", pdf_utf16be_hex(&heading));
    stream.push_str("0 -18 Td\n");
    for line in lines {
        let _ = writeln!(stream, "<{}> Tj", pdf_utf16be_hex(line));
        stream.push_str("0 -14 Td\n");
    }
    stream.push_str("ET");
    stream
}

fn wrap_pdf_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.lines() {
        let mut current = String::new();
        for character in raw_line.chars() {
            current.push(character);
            if current.chars().count() >= max_chars {
                lines.push(current.trim_end().to_string());
                current.clear();
            }
        }
        if !current.is_empty() || raw_line.is_empty() {
            lines.push(current.trim_end().to_string());
        }
    }
    lines
}

fn pdf_utf16be_hex(value: &str) -> String {
    let mut output = String::with_capacity(value.len() * 4 + 4);
    output.push_str("FEFF");
    for unit in value.encode_utf16() {
        let _ = write!(output, "{unit:04X}");
    }
    output
}

fn handle_save_download_message_with_event(body: &str) -> (String, DownloadEvent) {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(message) => message,
        Err(error) => {
            return (
                ipc_error(None, format!("ignored invalid download IPC: {error}")),
                DownloadEvent {
                    kind: DownloadEventKind::Failed,
                    status: "下载失败".to_string(),
                    path: None,
                    url: None,
                    bytes: None,
                    success: false,
                },
            );
        }
    };
    let id = ipc_message_id(&message);
    match save_download_payload(&message) {
        Ok(payload) => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let bytes = payload.get("bytes").and_then(Value::as_u64);
            (
                ipc_ok(id.as_deref(), payload),
                DownloadEvent {
                    kind: DownloadEventKind::Completed,
                    status: "下载完成".to_string(),
                    path,
                    url: None,
                    bytes,
                    success: true,
                },
            )
        }
        Err(error) => (
            ipc_error(id.as_deref(), format!("{error:#}")),
            DownloadEvent {
                kind: DownloadEventKind::Failed,
                status: "下载失败".to_string(),
                path: None,
                url: None,
                bytes: None,
                success: false,
            },
        ),
    }
}

fn handle_download_url_message_with_event(
    body: &str,
    proxy: Option<&ProxySettings>,
    expected_token: Option<&str>,
) -> (String, DownloadEvent) {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(message) => message,
        Err(error) => {
            return (
                ipc_error(None, format!("ignored invalid URL download IPC: {error}")),
                DownloadEvent {
                    kind: DownloadEventKind::Failed,
                    status: "下载失败".to_string(),
                    path: None,
                    url: None,
                    bytes: None,
                    success: false,
                },
            );
        }
    };
    let id = ipc_message_id(&message);
    let request_url = download_url_from_message(&message).ok().map(str::to_string);
    if let Err(error) = verify_download_ipc_token(&message, expected_token) {
        return (
            ipc_error(id.as_deref(), format!("{error:#}")),
            DownloadEvent {
                kind: DownloadEventKind::Failed,
                status: "下载失败".to_string(),
                path: None,
                url: request_url,
                bytes: None,
                success: false,
            },
        );
    }
    match download_url_payload(&message, proxy) {
        Ok(payload) => {
            let path = payload
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from);
            let bytes = payload.get("bytes").and_then(Value::as_u64);
            (
                ipc_ok(id.as_deref(), payload),
                DownloadEvent {
                    kind: DownloadEventKind::Completed,
                    status: "下载完成".to_string(),
                    path,
                    url: request_url,
                    bytes,
                    success: true,
                },
            )
        }
        Err(error) => (
            ipc_error(id.as_deref(), format!("{error:#}")),
            DownloadEvent {
                kind: DownloadEventKind::Failed,
                status: "下载失败".to_string(),
                path: None,
                url: request_url,
                bytes: None,
                success: false,
            },
        ),
    }
}

fn verify_download_ipc_token(message: &Value, expected_token: Option<&str>) -> Result<()> {
    let expected_token =
        expected_token.context("download URL IPC is not allowed from this view")?;
    let token = message
        .get("payload")
        .and_then(|payload| payload.get("token"))
        .and_then(Value::as_str)
        .context("download URL IPC token is missing")?;
    if token != expected_token {
        anyhow::bail!("download URL IPC token is invalid");
    }
    Ok(())
}

fn save_download_payload(message: &Value) -> Result<Value> {
    let payload = message
        .get("payload")
        .context("saveDownload payload is missing")?;
    let filename = payload
        .get("filename")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("download");
    let content_base64 = payload
        .get("content_base64")
        .and_then(Value::as_str)
        .context("download content is missing")?;
    let content_base64 = content_base64
        .split_once(',')
        .map(|(_, body)| body)
        .unwrap_or(content_base64);
    let bytes = BASE64
        .decode(content_base64)
        .context("download content is not valid base64")?;
    let path = write_download_bytes(filename, &bytes)?;

    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
    }))
}

fn download_url_payload(message: &Value, proxy: Option<&ProxySettings>) -> Result<Value> {
    let payload = message
        .get("payload")
        .context("downloadUrl payload is missing")?;
    let url = download_url_from_message(message)?;
    let filename = payload
        .get("filename")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| filename_from_url(url));
    let client = download_http_client(proxy)?;
    let response = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("could not download {url}"))?;
    let filename = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|header| header.to_str().ok())
        .and_then(filename_from_content_disposition)
        .unwrap_or(filename);
    let bytes = response.bytes().context("could not read downloaded file")?;
    let path = write_download_bytes(&filename, &bytes)?;

    Ok(json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
    }))
}

fn download_url_from_message(message: &Value) -> Result<&str> {
    let url = message
        .get("payload")
        .and_then(|payload| payload.get("url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .context("download URL is missing")?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        anyhow::bail!("download URL must use http or https");
    }
    Ok(url)
}

fn download_http_client(proxy: Option<&ProxySettings>) -> Result<Client> {
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("EasyGPT/", env!("CARGO_PKG_VERSION"), " download"));

    if let Some(proxy_settings) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url(proxy_settings)?)?);
    }

    builder
        .build()
        .context("could not build download HTTP client")
}

fn filename_from_url(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .filter(|segment| !segment.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "download".to_string())
}

fn filename_from_content_disposition(value: &str) -> Option<String> {
    value.split(';').find_map(|part| {
        let part = part.trim();
        let filename = part
            .strip_prefix("filename*=")
            .or_else(|| part.strip_prefix("filename="))?;
        let filename = filename
            .trim_matches('"')
            .trim_start_matches("UTF-8''")
            .trim();
        (!filename.is_empty()).then(|| filename.to_string())
    })
}

fn write_download_bytes(filename: &str, bytes: &[u8]) -> Result<PathBuf> {
    let sanitized = sanitize_download_filename(filename);
    let destination = download_destination_for(Path::new(&sanitized));
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create download directory {}", parent.display()))?;
    }
    std::fs::write(&destination, bytes)
        .with_context(|| format!("could not write download {}", destination.display()))?;
    Ok(destination)
}

fn sanitize_download_filename(filename: &str) -> String {
    let candidate = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
        .trim();
    let mut sanitized = candidate
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'..='\u{1f}' => '_',
            character => character,
        })
        .collect::<String>();
    while sanitized.ends_with([' ', '.']) {
        sanitized.pop();
    }

    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if sanitized.is_empty()
        || reserved
            .iter()
            .any(|reserved| sanitized.eq_ignore_ascii_case(reserved))
    {
        sanitized = "download".to_string();
    }

    sanitized
}

fn reveal_download_path(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let target = if path.exists() {
        path
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .filter(|parent| parent.exists())
            .context("download file or parent directory does not exist")?
    };

    open_path_in_file_manager(&target)
}

fn reveal_download_folder(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let folder = if path.is_dir() {
        path
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .filter(|parent| parent.exists())
            .context("download parent directory does not exist")?
    };

    open_path_in_file_manager(&folder)
}

#[cfg(windows)]
fn open_path_in_file_manager(path: &Path) -> Result<()> {
    let status = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            wide_null("open").as_ptr(),
            wide_null(&path.display().to_string()).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if (status as isize) <= 32 {
        anyhow::bail!(
            "could not open {}: ShellExecuteW returned {status:p}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_path_in_file_manager(path: &Path) -> Result<()> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .with_context(|| format!("could not open {}", path.display()))?;
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn open_path_in_file_manager(path: &Path) -> Result<()> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .with_context(|| format!("could not open {}", path.display()))?;
    Ok(())
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
        "openDownloadPath" => {
            let path = message
                .get("payload")
                .and_then(|payload| payload.get("path"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            Some(if path.is_empty() {
                Err(anyhow::anyhow!("download path is missing"))
            } else {
                Ok(ShellCommand::OpenDownloadPath(PathBuf::from(path)))
            })
        }
        "openDownloadFolder" => {
            let path = message
                .get("payload")
                .and_then(|payload| payload.get("path"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            Some(if path.is_empty() {
                Err(anyhow::anyhow!("download path is missing"))
            } else {
                Ok(ShellCommand::OpenDownloadFolder(PathBuf::from(path)))
            })
        }
        "openDownloadManager" => Some(Ok(ShellCommand::OpenDownloadManager)),
        "closeDownloadManager" => Some(Ok(ShellCommand::CloseDownloadManager)),
        "clearCompletedDownloads" => Some(Ok(ShellCommand::ClearCompletedDownloads)),
        "deleteDownloadRecord" => {
            let id = message
                .get("payload")
                .and_then(|payload| payload.get("id"))
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some(if id == 0 {
                Err(anyhow::anyhow!("download record id is missing"))
            } else {
                Ok(ShellCommand::DeleteDownloadRecord(id))
            })
        }
        "openDownloadSettings" => Some(Ok(ShellCommand::OpenDownloadSettings)),
        "measureLatency" => {
            let site = message
                .get("payload")
                .and_then(|payload| payload.get("site"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(
                AiSite::from_key(site)
                    .map(ShellCommand::MeasureLatency)
                    .ok_or_else(|| anyhow::anyhow!("unknown latency site: {site}")),
            )
        }
        "exportConversation" => {
            let format = message
                .get("payload")
                .and_then(|payload| payload.get("format"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(
                ExportFormat::from_key(format)
                    .map(ShellCommand::ExportConversation)
                    .ok_or_else(|| anyhow::anyhow!("unknown export format: {format}")),
            )
        }
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

  function notifyDownload(status, path, success = true) {
    if (window.parent && typeof window.parent.__aiClientNotifyDownload === 'function') {
      window.parent.__aiClientNotifyDownload({ status, path, success });
    }
  }

  async function readDownloadUrl(url) {
    const response = await fetch(url);
    if (!response.ok && !url.startsWith('blob:') && !url.startsWith('data:')) {
      throw new Error(`下载请求失败：${response.status}`);
    }
    const blob = await response.blob();
    const buffer = await blob.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    let binary = '';
    const chunkSize = 0x8000;
    for (let offset = 0; offset < bytes.length; offset += chunkSize) {
      binary += String.fromCharCode.apply(null, bytes.subarray(offset, offset + chunkSize));
    }
    return btoa(binary);
  }

  function installDownloadFallback() {
    document.addEventListener('click', async event => {
      const link = event.target && event.target.closest ? event.target.closest('a[download]') : null;
      if (!link || !link.href) return;
      const href = link.href;
      if (!href.startsWith('blob:') && !href.startsWith('data:')) return;
      if (!canUseNativeIpc()) return;
      event.preventDefault();
      event.stopPropagation();
      const filename = link.getAttribute('download') || 'download';
      try {
        notifyDownload('下载已开始', filename, true);
        const content_base64 = await readDownloadUrl(href);
        const result = await sendCommand('saveDownload', { filename, content_base64 });
        notifyDownload('下载完成', result.path, true);
      } catch (error) {
        notifyDownload(error.message || '下载失败', filename, false);
      }
    }, true);
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
      .cgpt-client-radio-row { display: flex; align-items: center; gap: 18px; flex-wrap: wrap; margin: 8px 0; }
      .cgpt-client-radio-label { display: inline-flex !important; align-items: center; gap: 7px; margin: 0 !important; }
      .cgpt-client-radio-label input { width: auto !important; min-height: auto !important; }
      .cgpt-client-path-row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 8px; align-items: end; }
      .cgpt-client-path-row label { margin: 0 !important; }
      .cgpt-client-download-hint { margin: 5px 0 8px; color: #666; font-size: 12px; word-break: break-all; }
      .cgpt-client-download-tools { display: flex; align-items: flex-end; justify-content: flex-end; gap: 8px; margin: 8px 0; }
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

        <h3 data-section="downloads">下载设置</h3>
        <div class="cgpt-client-radio-row" aria-label="下载内容保存位置">
          <label class="cgpt-client-radio-label">
            <input data-field="download_save_mode" type="radio" name="download-save-mode" value="last_dir" />
            使用上次下载目录
          </label>
          <label class="cgpt-client-radio-label">
            <input data-field="download_save_mode" type="radio" name="download-save-mode" value="fixed" />
            指定目录
          </label>
        </div>
        <div class="cgpt-client-path-row">
          <label>下载内容保存位置
            <input data-field="download_fixed_dir" type="text" placeholder="data/Downloads" />
          </label>
          <button class="cgpt-client-btn" type="button" data-action="choose-download-dir">更改...</button>
        </div>
        <div class="cgpt-client-download-hint">上次下载目录：<span data-view="download_last_dir">暂无</span></div>
        <div class="cgpt-client-grid">
          <label>下载记录保留
            <input data-field="download_max_records" type="number" min="50" max="5000" step="50" />
          </label>
          <div class="cgpt-client-download-tools">
            <button class="cgpt-client-btn" type="button" data-action="reset-download-dir">恢复默认目录</button>
          </div>
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
      download_save_modes: [...panel.querySelectorAll('[data-field="download_save_mode"]')],
      download_fixed_dir: panel.querySelector('[data-field="download_fixed_dir"]'),
      download_max_records: panel.querySelector('[data-field="download_max_records"]'),
    };
    const views = {
      runtime: panel.querySelector('[data-view="runtime"]'),
      current: panel.querySelector('[data-view="current"]'),
      nodes: panel.querySelector('[data-view="nodes"]'),
      log: panel.querySelector('#chatgpt-client-log'),
      status: panel.querySelector('#chatgpt-client-settings-status'),
      downloadLastDir: panel.querySelector('[data-view="download_last_dir"]'),
    };
    let subscriptions = [];
    let activeSubscriptionId = '';
    let downloadSettings = normalizeDownloadSettings(initialSettings.downloads || {});

    function setStatus(text) { views.status.textContent = text || ''; }
    function normalizeDownloadSettings(downloads = {}) {
      const maxRecords = Number(downloads.max_records ?? 500);
      return {
        save_mode: downloads.save_mode === 'last_dir' ? 'last_dir' : 'fixed',
        fixed_dir: String(downloads.fixed_dir || 'data/Downloads'),
        last_dir: String(downloads.last_dir || ''),
        ask_each_time: Boolean(downloads.ask_each_time),
        max_records: Number.isFinite(maxRecords) ? Math.min(5000, Math.max(50, Math.round(maxRecords))) : 500,
      };
    }
    function selectedDownloadSaveMode() {
      return fields.download_save_modes.find(input => input.checked)?.value || downloadSettings.save_mode || 'fixed';
    }
    function syncDownloadFields() {
      const mode = downloadSettings.save_mode === 'last_dir' ? 'last_dir' : 'fixed';
      fields.download_save_modes.forEach(input => { input.checked = input.value === mode; });
      fields.download_fixed_dir.value = downloadSettings.fixed_dir || 'data/Downloads';
      fields.download_fixed_dir.disabled = mode === 'last_dir';
      fields.download_max_records.value = String(downloadSettings.max_records || 500);
      views.downloadLastDir.textContent = downloadSettings.last_dir || '暂无';
      const chooseButton = panel.querySelector('[data-action="choose-download-dir"]');
      if (chooseButton) chooseButton.disabled = mode === 'last_dir';
    }
    function setDownloadSettings(downloads) {
      downloadSettings = normalizeDownloadSettings(downloads || {});
      syncDownloadFields();
    }
    function readDownloadSettings() {
      const maxRecords = Number(fields.download_max_records.value || downloadSettings.max_records || 500);
      downloadSettings = normalizeDownloadSettings({
        ...downloadSettings,
        save_mode: selectedDownloadSaveMode(),
        fixed_dir: fields.download_fixed_dir.value.trim() || 'data/Downloads',
        max_records: maxRecords,
      });
      return { ...downloadSettings };
    }
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
      setDownloadSettings(settings.downloads || initialSettings.downloads || {});
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
        },
        downloads: readDownloadSettings(),
      };
    }
    window.__chatgptClientUpdateDownloadSettings = (updates = {}) => {
      setDownloadSettings({ ...downloadSettings, ...updates });
    };
    window.__chatgptClientOpenSettings = (section) => {
      panel.hidden = false;
      if (section === 'downloads') {
        panel.querySelector('[data-section="downloads"]')?.scrollIntoView({ block: 'start' });
        fields.download_fixed_dir?.focus();
      }
    };
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
    fields.download_save_modes.forEach(input => {
      input.addEventListener('change', () => {
        downloadSettings.save_mode = selectedDownloadSaveMode();
        syncDownloadFields();
      });
    });
    fields.download_fixed_dir.addEventListener('input', () => {
      downloadSettings.fixed_dir = fields.download_fixed_dir.value;
    });
    fields.download_max_records.addEventListener('input', () => {
      downloadSettings.max_records = Number(fields.download_max_records.value || 500);
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
        } else if (action === 'choose-download-dir') {
          const current = fields.download_fixed_dir.value.trim() || 'data/Downloads';
          const next = window.prompt('请输入下载内容保存位置', current);
          if (next != null) {
            fields.download_fixed_dir.value = next.trim() || current;
            downloadSettings.fixed_dir = fields.download_fixed_dir.value;
            downloadSettings.save_mode = 'fixed';
            syncDownloadFields();
            setStatus('下载目录已更新，保存设置后生效');
          }
        } else if (action === 'reset-download-dir') {
          downloadSettings.fixed_dir = 'data/Downloads';
          downloadSettings.save_mode = 'fixed';
          syncDownloadFields();
          setStatus('已恢复默认下载目录，保存设置后生效');
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
  installDownloadFallback();
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

fn download_destination_for(suggested_path: &Path) -> PathBuf {
    let settings = load_settings();
    download_destination_for_with_settings(suggested_path, &settings.downloads)
}

fn download_destination_for_with_settings(
    suggested_path: &Path,
    settings: &DownloadSettings,
) -> PathBuf {
    let filename = cross_platform_file_name(suggested_path.as_os_str()).unwrap_or("download");
    let mut destination = resolve_download_directory(settings);
    if let Err(error) = ensure_download_directory_writable(&destination) {
        eprintln!(
            "could not create download directory {}: {error}",
            destination.display()
        );
        destination = user_download_dir().join("EasyGPT");
        let _ = std::fs::create_dir_all(&destination);
    }
    destination.push(filename);
    unique_download_path(destination)
}

fn cross_platform_file_name(path: &std::ffi::OsStr) -> Option<&str> {
    path.to_str()
        .and_then(|value| value.rsplit(['/', '\\']).next())
        .filter(|name| !name.trim().is_empty())
}

fn resolve_download_directory(settings: &DownloadSettings) -> PathBuf {
    let fixed = settings.fixed_dir.trim();
    let base_dir = chatgpt_webview_client::app_data_dir()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::env::temp_dir()));

    match settings.save_mode {
        DownloadSaveMode::LastDir if !settings.last_dir.trim().is_empty() => {
            PathBuf::from(settings.last_dir.trim())
        }
        _ if fixed.is_empty() => base_dir.join("data").join("Downloads"),
        _ => {
            let path = PathBuf::from(fixed);
            if path.is_absolute() {
                path
            } else {
                base_dir.join(path)
            }
        }
    }
}

fn ensure_download_directory_writable(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)
        .with_context(|| format!("could not create download directory {}", path.display()))?;
    let probe = path.join(".easygpt-write-test");
    std::fs::write(&probe, b"probe")
        .with_context(|| format!("could not probe download directory {}", path.display()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

fn user_download_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(std::env::temp_dir)
}

fn unique_download_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }

    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("download");
    let extension = path.extension().and_then(|extension| extension.to_str());

    for counter in 1..10_000 {
        let filename = match extension {
            Some(extension) if !extension.is_empty() => {
                format!("{stem} ({counter}).{extension}")
            }
            _ => format!("{stem} ({counter})"),
        };
        let candidate = parent.join(filename);
        if !candidate.exists() {
            return candidate;
        }
    }

    path
}

fn download_notification_script(status: &str, path: Option<&Path>, success: bool) -> String {
    let payload = json!({
        "status": status,
        "path": path.map(|path| path.display().to_string()),
        "success": success,
    });
    format!("window.__aiClientNotifyDownload && window.__aiClientNotifyDownload({payload});")
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
        AiSite, ExportFormat, ShellCommand, content_bounds, download_interceptor_script,
        export_conversation_payload, export_conversation_script, export_markdown_document,
        export_pdf_document, parse_shell_command, runtime_ready_script_for_site,
        settings_button_script, site_initial_url, top_shell_html, top_shell_url,
        unique_download_path,
    };
    use chatgpt_webview_client::{
        AppSettings, CHATGPT_URL, DownloadSaveMode, DownloadSettings, ProxyMode,
    };
    use serde_json::json;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        thread,
    };

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
    fn unique_download_path_keeps_unused_filename() {
        let path = PathBuf::from(r"C:\Users\tester\Downloads\report.pdf");

        assert_eq!(unique_download_path(path.clone()), path);
    }

    #[test]
    fn unique_download_path_adds_counter_for_existing_file() {
        let dir =
            std::env::temp_dir().join(format!("easygpt-download-test-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("report.pdf");
        fs::write(&path, b"existing").expect("write temp file");

        assert_eq!(unique_download_path(path), dir.join("report (1).pdf"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn download_destination_uses_suggested_filename() {
        let destination = super::download_destination_for(PathBuf::from("report.pdf").as_path());

        assert_eq!(
            destination.file_name().and_then(|name| name.to_str()),
            Some("report.pdf")
        );
        assert!(destination.is_absolute());
    }

    #[test]
    fn download_destination_uses_windows_basename_on_unix_hosts() {
        let destination = super::download_destination_for(
            PathBuf::from(r"C:\Users\tester\Downloads\report.pdf").as_path(),
        );

        assert_eq!(
            destination.file_name().and_then(|name| name.to_str()),
            Some("report.pdf")
        );
    }

    #[test]
    fn download_destination_respects_fixed_absolute_directory() {
        let dir = std::env::temp_dir().join(format!(
            "easygpt-fixed-download-test-{}",
            std::process::id()
        ));
        let settings = DownloadSettings {
            save_mode: DownloadSaveMode::Fixed,
            fixed_dir: dir.display().to_string(),
            ..Default::default()
        };

        let destination = super::download_destination_for_with_settings(
            PathBuf::from("report.pdf").as_path(),
            &settings,
        );

        assert!(destination.starts_with(&dir));
        assert_eq!(
            destination.file_name().and_then(|name| name.to_str()),
            Some("report.pdf")
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn download_destination_respects_last_directory_mode() {
        let dir =
            std::env::temp_dir().join(format!("easygpt-last-download-test-{}", std::process::id()));
        let settings = DownloadSettings {
            save_mode: DownloadSaveMode::LastDir,
            last_dir: dir.display().to_string(),
            ..Default::default()
        };

        let destination = super::download_destination_for_with_settings(
            PathBuf::from("report.pdf").as_path(),
            &settings,
        );

        assert!(destination.starts_with(&dir));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn content_initialization_script_installs_client_side_download_fallback() {
        let script = download_interceptor_script("test-token");

        assert!(script.contains("__chatgptClientDownloadInterceptorInstalled"));
        assert!(script.contains("sendDownloadCommand('saveDownload', payload)"));
        assert!(script.contains("const downloadIpcToken = \"test-token\";"));
        assert!(script.contains("content_base64"));
        assert!(script.contains("getAttribute('download')"));
        assert!(script.contains("href.startsWith('blob:')"));
        assert!(script.contains("href.startsWith('data:')"));
        assert!(script.contains("window.open = function"));
    }

    #[test]
    fn download_interceptor_supports_file_system_access_api_fallback() {
        let script = download_interceptor_script("test-token");

        assert!(script.contains("window.showSaveFilePicker"));
        assert!(script.contains("__aiClientNativeShowSaveFilePicker"));
        assert!(script.contains("createWritable"));
        assert!(script.contains("async close() { return flush(); }"));
        assert!(script.contains("navigator.msSaveBlob"));
    }

    #[test]
    fn save_download_payload_writes_base64_content() {
        let message = json!({
            "id": "download-1",
            "type": "saveDownload",
            "payload": {
                "filename": "codex-download-test.txt",
                "content_base64": "aGVsbG8="
            }
        });

        let payload = super::save_download_payload(&message).expect("download should save");
        let path = PathBuf::from(payload["path"].as_str().expect("path should be returned"));
        let bytes = fs::read(&path).expect("saved file should exist");

        assert_eq!(bytes, b"hello");
        assert_eq!(payload["bytes"], 5);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_download_payload_accepts_data_url_prefixed_base64() {
        let message = json!({
            "type": "saveDownload",
            "payload": {
                "filename": "easygpt-data-url-test.txt",
                "content_base64": "data:text/plain;base64,aGVsbG8="
            }
        });

        let payload = super::save_download_payload(&message).expect("download should save");
        let path = PathBuf::from(payload["path"].as_str().expect("path should be returned"));

        assert_eq!(fs::read(&path).expect("saved file should exist"), b"hello");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_download_payload_sanitizes_windows_forbidden_filename_chars() {
        let message = json!({
            "type": "saveDownload",
            "payload": {
                "filename": r#"folder\bad:name?.txt"#,
                "content_base64": "aGVsbG8="
            }
        });

        let payload = super::save_download_payload(&message).expect("download should save");
        let path = PathBuf::from(payload["path"].as_str().expect("path should be returned"));
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");

        assert!(filename.starts_with("bad_name_"));
        assert!(!filename.contains(':'));
        assert!(!filename.contains('?'));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn save_download_message_reports_completion_event() {
        let message = json!({
            "id": "download-2",
            "type": "saveDownload",
            "payload": {
                "filename": "codex-download-event-test.txt",
                "content_base64": "aGVsbG8="
            }
        });

        let (response, event) =
            super::handle_save_download_message_with_event(&message.to_string());
        let parsed: serde_json::Value = serde_json::from_str(&response).expect("response is json");
        let path = event.path.clone().expect("event should include saved path");

        assert_eq!(parsed["ok"], true);
        assert_eq!(event.status, "下载完成");
        assert!(event.success);
        assert!(path.exists());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn download_url_message_saves_http_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().expect("server address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/plain\r\n\r\nhello",
                )
                .expect("write response");
        });
        let url = format!("http://{addr}/generated.txt");
        let message = json!({
            "id": "download-url-1",
            "type": "downloadUrl",
            "payload": {
                "url": url,
                "filename": "generated.txt",
                "token": "download-token"
            }
        });

        let (response, event) = super::handle_download_url_message_with_event(
            &message.to_string(),
            None,
            Some("download-token"),
        );
        let parsed: serde_json::Value = serde_json::from_str(&response).expect("response is json");
        let path = event.path.clone().expect("event should include saved path");

        assert_eq!(parsed["ok"], true);
        assert_eq!(event.status, "下载完成");
        assert_eq!(event.url.as_deref(), Some(url.as_str()));
        assert_eq!(fs::read(&path).expect("saved file should exist"), b"hello");

        let _ = fs::remove_file(path);
        server.join().expect("server exits");
    }

    #[test]
    fn download_url_message_rejects_missing_ipc_token() {
        let message = json!({
            "id": "download-url-denied",
            "type": "downloadUrl",
            "payload": {
                "url": "http://127.0.0.1/generated.txt",
                "filename": "generated.txt"
            }
        });

        let (response, event) = super::handle_download_url_message_with_event(
            &message.to_string(),
            None,
            Some("download-token"),
        );
        let parsed: serde_json::Value = serde_json::from_str(&response).expect("response is json");

        assert_eq!(parsed["ok"], false);
        assert!(
            parsed["error"]
                .as_str()
                .expect("error should be returned")
                .contains("download URL IPC token is missing")
        );
        assert_eq!(event.status, "下载失败");
        assert!(!event.success);
    }

    #[test]
    fn download_diagnostic_ipc_reports_visible_error() {
        let event = super::parse_download_diagnostic_event(
            r#"{"type":"downloadDiagnostic","payload":{"level":"error","message":"script failed"}}"#,
        )
        .expect("diagnostic event");

        assert_eq!(event.status, "下载诊断：script failed");
        assert!(!event.success);
    }

    #[test]
    fn download_history_records_started_and_completed_items() {
        let mut history = super::DownloadHistory::default();
        let path = PathBuf::from(r"C:\Users\tester\Downloads\report.docx");

        history.record(super::DownloadEvent {
            kind: super::DownloadEventKind::Started,
            status: "下载已开始".to_string(),
            path: Some(path.clone()),
            url: Some("https://example.com/report.docx".to_string()),
            bytes: None,
            success: true,
        });
        history.record(super::DownloadEvent {
            kind: super::DownloadEventKind::Completed,
            status: "下载完成".to_string(),
            path: Some(path.clone()),
            url: Some("https://example.com/report.docx".to_string()),
            bytes: Some(42),
            success: true,
        });

        let payload = history.payload();
        assert_eq!(payload["downloads"][0]["filename"], "report.docx");
        assert_eq!(payload["downloads"][0]["status"], "completed");
        assert_eq!(payload["downloads"][0]["path"], path.display().to_string());
        assert_eq!(payload["downloads"][0]["bytes"], 42);
    }

    #[test]
    fn download_history_store_round_trips_completed_records() {
        let mut history = super::DownloadHistory::new(500);
        let path = PathBuf::from(r"C:\Users\tester\Downloads\report.xlsx");
        history.record(super::DownloadEvent {
            kind: super::DownloadEventKind::Completed,
            status: "下载完成".to_string(),
            path: Some(path.clone()),
            url: Some("https://example.com/report.xlsx".to_string()),
            bytes: Some(12),
            success: true,
        });

        let store = super::download_history_store(&history);
        let encoded = serde_json::to_string(&store).expect("store should serialize");
        let decoded = serde_json::from_str::<super::DownloadHistoryStore>(&encoded)
            .expect("store should deserialize");
        let restored = super::download_history_from_store(decoded, 500);

        assert_eq!(restored.records.len(), 1);
        assert_eq!(restored.records[0].filename, "report.xlsx");
        assert_eq!(
            restored.records[0].status,
            super::DownloadRecordStatus::Completed
        );
    }

    #[test]
    fn download_history_load_marks_previous_started_as_failed() {
        let store = super::DownloadHistoryStore {
            version: 1,
            next_id: 1,
            records: vec![super::DownloadRecord {
                id: 1,
                filename: "unfinished.zip".to_string(),
                status: super::DownloadRecordStatus::Started,
                path: Some(PathBuf::from(r"C:\Downloads\unfinished.zip")),
                url: None,
                bytes: None,
                message: "下载已开始".to_string(),
                timestamp_ms: 1,
            }],
        };

        let history = super::download_history_from_store(store, 500);

        assert_eq!(
            history.records[0].status,
            super::DownloadRecordStatus::Failed
        );
        assert_eq!(history.records[0].message, "上次退出时下载未完成");
    }

    #[test]
    fn download_history_load_caps_records() {
        let records = (0..3)
            .map(|id| super::DownloadRecord {
                id,
                filename: format!("file-{id}.txt"),
                status: super::DownloadRecordStatus::Completed,
                path: None,
                url: None,
                bytes: None,
                message: "下载完成".to_string(),
                timestamp_ms: u128::from(id),
            })
            .collect();
        let store = super::DownloadHistoryStore {
            version: 1,
            next_id: 3,
            records,
        };

        let history = super::download_history_from_store(store, 2);

        assert_eq!(history.records.len(), 2);
    }

    #[test]
    fn download_history_clear_completed_keeps_failed_records() {
        let mut history = super::DownloadHistory::new(500);
        history.records = vec![
            super::DownloadRecord {
                id: 1,
                filename: "done.txt".to_string(),
                status: super::DownloadRecordStatus::Completed,
                path: None,
                url: None,
                bytes: None,
                message: "下载完成".to_string(),
                timestamp_ms: 1,
            },
            super::DownloadRecord {
                id: 2,
                filename: "failed.txt".to_string(),
                status: super::DownloadRecordStatus::Failed,
                path: None,
                url: None,
                bytes: None,
                message: "下载失败".to_string(),
                timestamp_ms: 2,
            },
            super::DownloadRecord {
                id: 3,
                filename: "diagnostic.txt".to_string(),
                status: super::DownloadRecordStatus::Diagnostic,
                path: None,
                url: None,
                bytes: None,
                message: "下载诊断".to_string(),
                timestamp_ms: 3,
            },
        ];

        history.clear_completed();

        assert_eq!(history.records.len(), 2);
        assert!(
            history
                .records
                .iter()
                .all(|record| record.status != super::DownloadRecordStatus::Completed)
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
    fn top_shell_html_exposes_download_toast_api() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("class=\"toast-region\""));
        assert!(html.contains("window.__aiClientNotifyDownload"));
        assert!(html.contains("detail.dataset.action = 'openDownloadPath';"));
        assert!(html.contains("sendCommand('openDownloadPath'"));
    }

    #[test]
    fn top_shell_html_exposes_download_self_test_button() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(!html.contains("data-action=\"testDownload\""));
        assert!(!html.contains("下载自测"));
    }

    #[test]
    fn top_shell_download_manager_opens_native_window() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("data-action=\"openDownloadManager\""));
        assert!(html.contains("sendCommand('openDownloadManager')"));
        assert!(!html.contains("class=\"download-overlay\""));
        assert!(!html.contains("setDownloadManagerOpen"));
    }

    #[test]
    fn top_shell_html_exposes_latency_status_and_timer() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("data-latency-label"));
        assert!(html.contains("延时 --"));
        assert!(html.contains("measureLatency"));
        assert!(html.contains("setInterval(measureActiveLatency, 60000)"));
    }

    #[test]
    fn top_shell_html_does_not_probe_latency_during_first_second() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(!html.contains("setTimeout(measureActiveLatency, 800)"));
        assert!(html.contains("setTimeout(measureActiveLatency, 5000)"));
    }

    #[test]
    fn top_shell_html_latency_pill_has_stable_width() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("class=\"pill latency-pill\""));
        assert!(html.contains(".latency-pill {"));
        assert!(html.contains("min-width: 104px;"));
        assert!(html.contains("justify-content: center;"));
    }

    #[test]
    fn top_shell_html_exposes_download_manager_panel() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("data-action=\"openDownloadManager\""));
        assert!(!html.contains("data-download-filter"));
        assert!(!html.contains("data-download-list"));
        assert!(!html.contains("data-action=\"testDownload\""));
    }

    #[test]
    fn download_manager_html_wires_native_window_buttons() {
        let history = super::DownloadHistory::new(500);
        let html = super::download_manager_html(&history);

        assert!(html.contains("<title>下载管理</title>"));
        assert!(html.contains("data-download-filter"));
        assert!(html.contains("data-download-list"));
        assert!(html.contains("data-download-summary"));
        assert!(html.contains("data-action=\"openDownloadSettings\""));
        assert!(html.contains("data-action=\"clearCompletedDownloads\""));
        assert!(html.contains("data-action=\"closeDownloadManager\""));
        assert!(html.contains("data-action=\"openDownloadPath\""));
        assert!(html.contains("data-action=\"openDownloadFolder\""));
        assert!(html.contains("data-action=\"deleteDownloadRecord\""));
        assert!(html.contains("window.__aiClientSyncDownloads"));
        assert!(html.contains("sendCommand('closeDownloadManager')"));
    }

    #[test]
    fn top_shell_html_exposes_conversation_export_buttons() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains("data-action=\"exportMarkdown\""));
        assert!(html.contains("data-action=\"exportPdf\""));
        assert!(html.contains("exportConversation"));
        assert!(html.contains("title=\"导出 Markdown\""));
        assert!(html.contains("title=\"导出 PDF\""));
    }

    #[test]
    fn rendered_pdf_filename_uses_site_name() {
        assert_eq!(
            super::rendered_pdf_filename(AiSite::NotebookLm),
            "NotebookLM-page-export.pdf"
        );
    }

    #[test]
    fn top_shell_toast_is_visible_inside_fixed_toolbar_height() {
        let html = top_shell_html(AiSite::ChatGpt);

        assert!(html.contains(".toast-region { position: fixed; right: 12px; top: 8px;"));
        assert!(!html.contains("top: 60px"));
    }

    #[test]
    fn export_conversation_script_posts_markdown_to_rust() {
        let script = export_conversation_script(AiSite::ChatGpt, ExportFormat::Markdown);

        assert!(script.contains("type: 'exportConversation'"));
        assert!(script.contains("const format = 'markdown';"));
        assert!(script.contains("extractConversationMarkdown"));
        assert!(script.contains("data-message-author-role"));
    }

    #[test]
    fn export_markdown_document_includes_metadata_and_content() {
        let markdown = export_markdown_document("ChatGPT", "https://chatgpt.com/c/1", "hello");

        assert!(markdown.contains("# ChatGPT"));
        assert!(markdown.contains("来源：<https://chatgpt.com/c/1>"));
        assert!(markdown.contains("hello"));
    }

    #[test]
    fn export_pdf_document_creates_pdf_bytes() {
        let pdf = export_pdf_document("ChatGPT", "# ChatGPT\n\nhello");

        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("%%EOF"));
    }

    #[test]
    fn export_pdf_document_does_not_replace_chinese_with_question_marks() {
        let pdf = export_pdf_document("ChatGPT", "# ChatGPT\n\n## 用户\n\n你好，导出测试");
        let rendered = String::from_utf8_lossy(&pdf);

        assert!(!rendered.contains("??"));
        assert!(!rendered.contains("????"));
    }

    #[test]
    fn export_conversation_payload_writes_markdown_file() {
        let message = json!({
            "id": "export-1",
            "type": "exportConversation",
            "payload": {
                "format": "markdown",
                "site_title": "ChatGPT",
                "url": "https://chatgpt.com/c/1",
                "markdown": "## 用户\n\nhello"
            }
        });

        let payload = export_conversation_payload(&message).expect("export should save");
        let path = PathBuf::from(payload["path"].as_str().expect("path should be returned"));

        assert_eq!(payload["format"], "markdown");
        assert!(path.exists());
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("md")
        );
        assert!(
            fs::read_to_string(&path)
                .expect("markdown should be readable")
                .contains("## 用户")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn export_conversation_payload_writes_pdf_file() {
        let message = json!({
            "id": "export-2",
            "type": "exportConversation",
            "payload": {
                "format": "pdf",
                "site_title": "ChatGPT",
                "url": "https://chatgpt.com/c/1",
                "markdown": "## User\n\nhello"
            }
        });

        let payload = export_conversation_payload(&message).expect("export should save");
        let path = PathBuf::from(payload["path"].as_str().expect("path should be returned"));
        let bytes = fs::read(&path).expect("pdf should be readable");

        assert_eq!(payload["format"], "pdf");
        assert_eq!(
            path.extension().and_then(|value| value.to_str()),
            Some("pdf")
        );
        assert!(bytes.starts_with(b"%PDF-1.4"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn download_notification_script_escapes_payload() {
        let path = PathBuf::from(r#"C:\Users\tester\Downloads\a"b.txt"#);
        let script = super::download_notification_script("完成", Some(path.as_path()), true);

        assert!(script.contains("window.__aiClientNotifyDownload"));
        assert!(script.contains(r#"C:\\Users\\tester\\Downloads\\a\"b.txt"#));
    }

    #[test]
    fn top_shell_loads_from_non_opaque_origin_for_ipc() {
        let url = top_shell_url();

        assert!(url.starts_with("aiclient://"));
        assert!(url.contains("?v=download-manager-native-v2"));
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
    fn parse_shell_command_recognizes_open_download_path() {
        let command = parse_shell_command(
            r#"{"id":"5","type":"openDownloadPath","payload":{"path":"C:\\Users\\tester\\Downloads\\report.md"}}"#,
        )
        .expect("shell command")
        .expect("valid open path command");

        assert_eq!(
            command,
            ShellCommand::OpenDownloadPath(PathBuf::from(r#"C:\Users\tester\Downloads\report.md"#))
        );
    }

    #[test]
    fn parse_shell_command_recognizes_open_download_folder() {
        let command = parse_shell_command(
            r#"{"id":"6","type":"openDownloadFolder","payload":{"path":"C:\\Users\\tester\\Downloads\\report.md"}}"#,
        )
        .expect("shell command")
        .expect("valid open folder command");

        assert_eq!(
            command,
            ShellCommand::OpenDownloadFolder(PathBuf::from(
                r#"C:\Users\tester\Downloads\report.md"#
            ))
        );
    }

    #[test]
    fn parse_shell_command_recognizes_download_manager_commands() {
        assert_eq!(
            parse_shell_command(r#"{"id":"7","type":"openDownloadManager"}"#)
                .expect("shell command")
                .expect("valid command"),
            ShellCommand::OpenDownloadManager
        );
        assert_eq!(
            parse_shell_command(r#"{"id":"7","type":"closeDownloadManager"}"#)
                .expect("shell command")
                .expect("valid command"),
            ShellCommand::CloseDownloadManager
        );
        assert_eq!(
            parse_shell_command(r#"{"id":"8","type":"clearCompletedDownloads"}"#)
                .expect("shell command")
                .expect("valid command"),
            ShellCommand::ClearCompletedDownloads
        );
        assert_eq!(
            parse_shell_command(r#"{"id":"9","type":"deleteDownloadRecord","payload":{"id":12}}"#)
                .expect("shell command")
                .expect("valid command"),
            ShellCommand::DeleteDownloadRecord(12)
        );
        assert_eq!(
            parse_shell_command(r#"{"id":"10","type":"openDownloadSettings"}"#)
                .expect("shell command")
                .expect("valid command"),
            ShellCommand::OpenDownloadSettings
        );
    }

    #[test]
    fn parse_shell_command_recognizes_latency_measurement() {
        let command = parse_shell_command(
            r#"{"id":"9","type":"measureLatency","payload":{"site":"gemini"}}"#,
        )
        .expect("shell command")
        .expect("valid latency command");

        assert_eq!(command, ShellCommand::MeasureLatency(AiSite::Gemini));
    }

    #[test]
    fn parse_shell_command_recognizes_export_markdown() {
        let command = parse_shell_command(
            r#"{"id":"5","type":"exportConversation","payload":{"format":"markdown"}}"#,
        )
        .expect("shell command")
        .expect("valid export command");

        assert_eq!(
            command,
            ShellCommand::ExportConversation(ExportFormat::Markdown)
        );
    }

    #[test]
    fn parse_shell_command_recognizes_export_pdf() {
        let command = parse_shell_command(
            r#"{"id":"6","type":"exportConversation","payload":{"format":"pdf"}}"#,
        )
        .expect("shell command")
        .expect("valid export command");

        assert_eq!(command, ShellCommand::ExportConversation(ExportFormat::Pdf));
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
    fn settings_script_exposes_download_path_controls() {
        let script = settings_button_script(r#"{"proxy":{},"downloads":{}}"#);

        assert!(script.contains("下载内容保存位置"));
        assert!(script.contains("使用上次下载目录"));
        assert!(script.contains("data-field=\"download_save_mode\""));
        assert!(script.contains("data-field=\"download_fixed_dir\""));
        assert!(script.contains("data-field=\"download_max_records\""));
        assert!(script.contains("data-action=\"choose-download-dir\""));
        assert!(script.contains("window.__chatgptClientUpdateDownloadSettings"));
    }

    #[test]
    fn settings_script_intercepts_blob_and_data_download_links() {
        let script = download_interceptor_script("test-token");

        assert!(script.contains("__chatgptClientDownloadInterceptorInstalled"));
        assert!(script.contains("a[href]"));
        assert!(script.contains("href.startsWith('blob:')"));
        assert!(script.contains("href.startsWith('data:')"));
        assert!(script.contains("saveDownload"));
        assert!(script.contains("content_base64"));
    }

    #[test]
    fn download_interceptor_routes_regular_http_links_to_native_download() {
        let script = download_interceptor_script("test-token");

        assert!(script.contains("sendDownloadCommand('downloadUrl', Object.assign({}, payload"));
        assert!(script.contains("token: downloadIpcToken"));
        assert!(script.contains("fetch(url, { credentials: 'include' })"));
        assert!(script.contains("return sendDownloadUrl({ url, filename })"));
        assert!(script.contains("url.startsWith('http://')"));
        assert!(script.contains("url.startsWith('https://')"));
    }

    #[test]
    fn download_interceptor_handles_programmatic_anchor_clicks() {
        let script = download_interceptor_script("test-token");

        assert!(script.contains("HTMLAnchorElement.prototype.click"));
        assert!(script.contains("nativeAnchorClick"));
        assert!(script.contains("handleDownloadAnchor"));
        assert!(script.contains("return nativeAnchorClick.apply(this, arguments);"));
    }

    #[test]
    fn save_download_payload_writes_base64_file() {
        let message = json!({
            "type": "saveDownload",
            "payload": {
                "filename": "easygpt-test-download.txt",
                "content_base64": "aGVsbG8="
            }
        });

        let payload = super::save_download_payload(&message).expect("save download");
        let path = PathBuf::from(payload["path"].as_str().expect("path"));

        assert_eq!(fs::read(&path).expect("read saved file"), b"hello");
        let _ = fs::remove_file(path);
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
    fn runtime_failed_script_replaces_waiting_page_with_error() {
        let script = super::runtime_failed_script("missing mihomo");

        assert!(script.contains("内置代理启动失败"));
        assert!(script.contains("missing mihomo"));
        assert!(script.contains("resources/clash/mihomo"));
        assert!(script.contains("document.body.innerHTML = '';"));
    }

    #[test]
    fn startup_stage_has_user_facing_labels() {
        assert_eq!(super::StartupStage::all().len(), 10);
        assert_eq!(
            super::StartupStage::LoadSubscription.key(),
            "load_subscription"
        );
        assert_eq!(
            super::StartupStage::LoadSubscription.label(),
            "读取或更新订阅"
        );
        assert_eq!(
            super::StartupStage::WaitController.label(),
            "等待控制器就绪"
        );
        assert_eq!(super::StartupStage::Failed.key(), "failed");
    }

    #[test]
    fn startup_progress_script_updates_waiting_page() {
        let script = super::startup_progress_script(&super::StartupProgress {
            stage: super::StartupStage::StartMihomo,
            elapsed_secs: 8,
            message: Some("starting".to_string()),
        });

        assert!(script.contains("data-startup-stage"));
        assert!(script.contains("data-startup-elapsed"));
        assert!(script.contains("启动 mihomo"));
        assert!(script.contains("starting"));
    }

    #[test]
    fn initial_runtime_start_spawns_once_only_for_internal_clash() {
        assert!(super::should_spawn_initial_runtime_start(
            ProxyMode::InternalClash,
            false
        ));
        assert!(!super::should_spawn_initial_runtime_start(
            ProxyMode::InternalClash,
            true
        ));
        assert!(!super::should_spawn_initial_runtime_start(
            ProxyMode::System,
            false
        ));
    }

    #[test]
    fn waiting_page_keeps_elapsed_timer_without_native_events() {
        let url = super::waiting_page_url(AiSite::ChatGpt);
        let decoded = urlencoding::decode(
            url.strip_prefix("data:text/html;charset=utf-8,")
                .expect("waiting page should be a data URL"),
        )
        .expect("waiting page should decode");

        assert!(decoded.contains("const startupStartedAt = Date.now();"));
        assert!(decoded.contains("setInterval(() => {"));
        assert!(decoded.contains("data-startup-elapsed"));
    }

    #[test]
    fn waiting_page_bootstraps_initial_startup_stage() {
        let url = super::waiting_page_url(AiSite::ChatGpt);
        let decoded = urlencoding::decode(
            url.strip_prefix("data:text/html;charset=utf-8,")
                .expect("waiting page should be a data URL"),
        )
        .expect("waiting page should decode");

        assert!(decoded.contains("data-startup-stage>读取配置</strong>"));
        assert!(decoded.contains("data-startup-elapsed>0</span>s"));
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
        assert!(url.contains("data-startup-stage"));
        assert!(url.contains("%E8%B7%B3%E8%BF%87%E4%BB%A3%E7%90%86%E6%89%93%E5%BC%80"));
        assert!(url.contains("https%3A%2F%2Fchatgpt.com"));
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
