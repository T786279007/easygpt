# EasyGPT v1.0 产品性能与下载中心规格

## 1. 背景

EasyGPT 已经具备 Rust + Wry/WebView 单窗口客户端、便携数据目录、内置 mihomo/Clash、多 AI 页面切换、代理设置、节点选择、延时检测、下载记录、Markdown/PDF 导出和安装包打包能力。

后续重点不是继续堆按钮，而是把产品体验稳定下来：启动要可解释，代理要可恢复，页面要不卡，下载要可追踪，配置和登录态要尽量可迁移。

## 2. 总目标

1. 打开应用后尽快可用，不因为代理、订阅、节点或 WebView 生命周期卡死。
2. 内置代理只影响本应用，不影响系统其他软件。
3. ChatGPT、Gemini、NotebookLM、Google AI Studio 能在顶部切换，并尽量长期保持登录态。
4. 多订阅、多节点、延时检测、自动保存选择稳定可用。
5. 下载体验升级为浏览器式下载中心，支持保存路径设置和历史记录持久化。
6. 减少卡顿和内存泄露风险，提供显式内存清理和后台页面释放策略。
7. 出问题时用户能看到原因，开发者能拿到诊断信息。

## 3. 非目标

1. 不做完整浏览器，不支持任意网页标签管理。
2. 不做系统全局代理工具，不替代 Clash Verge、Clash for Windows 等软件。
3. 不承诺跨设备 100% 免登录，因为 Cookie 可能与设备、IP、系统密钥或浏览器指纹绑定。
4. 首轮不做 BT、磁力、断点续传、多线程下载。
5. 不为了视觉复杂度牺牲轻量和稳定。

## 4. 版本路线

### v0.10 稳定启动与代理状态机

解决“正在启动内置代理”无限等待、代理就绪但页面不跳转、外部 Clash 干扰、端口占用等问题。

交付：

- 启动状态机。
- 代理启动超时。
- 代理错误页。
- 运行态诊断。
- `RuntimeReady` 导航可靠性修复。
- mihomo 进程生命周期守护。
- 内置代理端口自愈。

### v0.11 下载中心与保存路径

把下载从“点了不知道去哪了”升级成浏览器式下载管理。

交付：

- 独立下载中心页面/弹窗。
- 下载历史持久化。
- 下载保存路径设置。
- 打开文件、打开目录、删除记录、清空已完成。
- 搜索下载记录。
- 失败原因显示。

### v0.12 性能与内存优化

减少卡顿和内存占用。

交付：

- 多页面懒加载。
- 后台 WebView 低内存策略。
- 后台页面释放策略。
- 节点测速限流。
- UI 更新节流。
- 内存清理按钮增强。

### v0.13 导出与迁移增强

补齐长期使用能力。

交付：

- Markdown 导出增强。
- 当前页面 PDF 导出。
- 长截图导出 PDF。
- 便携数据导入/导出。

## 5. 下载中心数据结构

下载历史保存到：

```text
data/downloads.json
```

记录字段建议：

```json
{
  "id": 1,
  "filename": "report.pdf",
  "path": "data/Downloads/report.pdf",
  "url": "https://example.com/report.pdf",
  "status": "completed",
  "bytes": 1024,
  "timestamp_ms": 1780000000000,
  "message": ""
}
```

历史记录默认保留 500 条，超过后删除最旧记录。

## 6. 下载设置

设置保存到 `settings.toml`：

```toml
[downloads]
save_mode = "fixed"
fixed_dir = "data/Downloads"
last_dir = ""
ask_each_time = false
max_records = 500
```

## 7. 性能要求

- 节点测速必须分批执行。
- UI 状态更新需要节流。
- 网络和文件操作不得阻塞 UI 线程。
- 隐藏页面可设置低内存状态。
- 用户关闭次要页面后应释放对应 WebView。

## 8. 诊断包要求

诊断包可放到：

```text
data/diagnostics/easygpt-diagnostic-YYYYMMDD-HHMMSS.zip
```

可包含：

- 基本版本信息。
- 设置文件脱敏副本。
- mihomo 日志尾部。
- 下载历史摘要。
- 当前代理状态。

不得包含：

- Cookie。
- LocalStorage。
- 完整 `WebView2Profile`。
- 未脱敏订阅 token。
- 代理 server/password/token 明文。

## 9. 对抗式审阅

- 如果安装到 `Program Files`，程序目录不可写，因此安装型包必须把数据目录放到用户目录。
- 订阅和日志可能含 token，导出诊断前必须脱敏。
- 长期使用后 `downloads.json` 可能过大，需要限制记录数。
- 下载中心不能依赖网页自己的下载按钮。
- 代理失败不能导致应用闪退，应进入错误页并给出可操作提示。

## 10. 验证

```powershell
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```
