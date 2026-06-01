# EasyGPT v1.0 产品性能与下载中心开发规格

## 1. 背景

EasyGPT 当前已经具备 Rust + Wry/WebView2 单窗口客户端、便携数据目录、内置 mihomo/Clash、多 AI 页面切换、代理设置、节点选择、延时检测、下载基础记录、导出 Markdown/PDF、安装包打包等能力。

后续版本的重点不应继续堆叠零散按钮，而应把产品体验稳定下来：启动要可解释，代理要可恢复，页面要不卡，下载要可追踪，配置和登录态要尽量可迁移。

本文档汇总前面所有讨论，作为下一轮开发的总规格。

## 2. 总目标

1. 打开 EXE 后尽快可用，不因代理、订阅、节点或 WebView 生命周期卡死。
2. 内置代理只影响本应用，不影响系统其他软件，也不被外部 Clash/cclash 干扰。
3. ChatGPT、Gemini、NotebookLM、Google AI Studio 能在顶部切换，并尽量长期保持登录态。
4. 多订阅、多节点、延时检测、自动保存选择都稳定可用。
5. 下载体验升级为浏览器式下载中心，支持保存路径设置和历史记录持久化。
6. 性能上减少卡顿和内存泄露风险，提供显式内存清理和后台页面释放策略。
7. 出问题时用户能看到原因，开发者能拿到诊断信息。

## 3. 非目标

1. 不做完整浏览器，不支持任意网页标签页管理。
2. 不做系统全局代理工具，不替代 Clash Verge、Clash for Windows 等软件。
3. 不承诺跨设备 100% 免登录，因为 ChatGPT/Google 的 Cookie 可能和设备、IP、系统密钥、浏览器指纹绑定。
4. 不在首轮实现复杂下载协议，例如 BT、磁力、断点续传多线程下载。
5. 不为了视觉复杂度牺牲轻量和稳定。

## 4. 版本路线

### v0.10 稳定启动与代理状态机

目标是解决“正在启动内置代理”无限等待、代理就绪但页面不跳转、外部 Clash 干扰、端口占用等问题。

交付内容：

- 启动状态机
- 代理启动超时
- 代理错误页
- 运行态诊断报告
- RuntimeReady 导航可靠性修复
- mihomo 进程生命周期守护
- 内置代理端口自愈

### v0.11 下载中心与保存路径

目标是把下载从“点了不知道去哪了”升级成浏览器式下载管理。

交付内容：

- 独立下载中心页面/弹窗
- 下载历史持久化
- 下载保存路径设置
- 打开文件、打开目录、删除记录、清空已完成
- 搜索下载记录
- 失败原因显示

### v0.12 性能与内存优化

目标是减少卡顿和内存占用。

交付内容：

- 多页面懒加载
- 后台 WebView 低内存策略
- 后台页面释放策略
- 节点测速限流
- UI 更新节流
- 内存清理按钮增强

### v0.13 导出与迁移增强

目标是补齐长期使用能力。

交付内容：

- Markdown 导出增强
- 当前页面 PDF 导出
- 长截图导出 PDF
- 便携数据导入/导出
- 配置迁移向导

### v1.0 产品化版本

目标是达到日常稳定使用标准。

交付内容：

- 自动更新
- GitHub Actions 打包
- 安装包标准化
- 托盘与快捷键
- 隐私模式
- 崩溃恢复和诊断导出

## 5. 启动状态机

### 5.1 状态定义

启动过程拆成明确状态：

1. `LoadSettings`：读取 `data/settings.toml`
2. `ResolveDataDirs`：确认 `data`、`WebView2Profile`、`clash` 目录
3. `ResolvePorts`：检查 mixed/controller 端口
4. `LoadSubscription`：读取缓存订阅或下载最新订阅
5. `BuildMihomoConfig`：生成 `data/clash/config.yaml`
6. `StartMihomo`：启动 bundled `mihomo.exe`
7. `WaitController`：等待 Controller API 可访问
8. `RestoreNode`：恢复上次选择的策略组和节点
9. `CheckConnectivity`：检测 ChatGPT 访问能力
10. `OpenInitialSite`：打开 ChatGPT 或当前选中的 AI 页面

### 5.2 UI 表现

等待页不再只显示“正在启动内置代理”，而是显示当前阶段：

```text
正在启动内置代理
当前步骤：等待 mihomo 控制器就绪
已用时：8s
```

如果超过 15 秒未完成，显示操作：

- 重试代理
- 跳过代理打开
- 打开设置
- 查看日志
- 导出诊断

### 5.3 技术设计

Rust 主进程维护 `StartupStage`：

```rust
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
```

通过 `UserEvent::StartupProgress` 向 shell WebView 或等待页广播状态。

等待页必须不依赖外网资源，使用本地 `data:` 或自定义协议页面。

## 6. 内置代理与订阅

### 6.1 代理模式

继续支持：

- System：使用系统代理
- Direct：直连
- InternalClash：只给本应用 WebView 使用内置 mihomo

InternalClash 必须满足：

- 不修改系统代理
- 不影响其他应用
- 不抢占外部 Clash/cclash 进程
- 只清理本程序目录下启动的 bundled mihomo

### 6.2 多订阅管理

设置结构保留：

```toml
[proxy]
mode = "internal_clash"
active_subscription_id = "sub-xxx"
auto_update_subscription = true
selected_group = "Zi You To"
selected_proxy = "新加坡"

[[proxy.subscriptions]]
id = "sub-xxx"
name = "默认订阅"
url = "https://..."
```

新增要求：

- 添加订阅不覆盖原订阅
- 切换订阅后清空旧节点选择
- 保存后立即刷新订阅和节点列表
- 下载失败时使用对应订阅的缓存
- 每个订阅独立缓存到 `data/clash/subscriptions/<id>.yaml`

### 6.3 节点选择

节点 UI 显示：

- 节点名
- 所属策略组
- 延迟
- 最近测速时间
- 失败原因
- 当前选中标记

节点操作：

- 选择节点
- 测速当前节点
- 测速全部节点
- 自动选择最快可用节点
- 保存当前节点，下次启动自动恢复

### 6.4 测速策略

测速必须限流：

- 当前节点每 60 秒自动检测一次
- 全量测速只在用户手动触发
- 并发数默认 4
- 单节点超时 5 秒
- 结果缓存 5 分钟

测速 URL：

- ChatGPT：`https://chatgpt.com/cdn-cgi/trace`
- Gemini：`https://gemini.google.com`
- NotebookLM：`https://notebooklm.google.com`
- AI Studio：`https://aistudio.google.com`

测速必须在 Rust 后台线程执行，不能在 WebView UI 线程执行。

## 7. 多页面与登录态

### 7.1 页面范围

顶部固定四个入口：

- ChatGPT
- Gemini
- NotebookLM
- Google AI Studio

### 7.2 WebView 策略

ChatGPT 默认启动。其他页面首次点击时懒加载。

页面状态：

- Active：当前显示
- Warm：最近使用，隐藏但保留 WebView
- Released：释放 WebView，下次重新创建

默认策略：

- ChatGPT 常驻
- 最近一个非 ChatGPT 页面保温
- 其他后台页面可释放

### 7.3 登录态

所有页面共享同一个便携 WebView2 Profile：

```text
data/WebView2Profile
```

需要在产品说明里明确：

- 同设备重启通常可以保持登录
- 换设备复制整个目录可迁移部分登录数据
- ChatGPT/Google 可能要求重新验证

## 8. 下载中心

### 8.1 入口

顶部工具栏保留下载图标。点击后打开居中的下载中心，不再只依赖右上角小面板。

下载中心尺寸：

- 默认宽度：900px
- 默认高度：640px
- 小窗口下使用 `calc(100vw - 32px)` 和 `calc(100vh - 32px)`

### 8.2 页面结构

顶部：

```text
下载                                    最小化  关闭
```

列表行参考截图设计：

```text
[文件图标] 文件名
          123 KB - 完成
                                  [打开] [文件夹] [删除]
```

底部：

```text
[清空已完成] [新建下载] [下载设置]        [搜索下载内容]
```

### 8.3 下载记录字段

新增文件：

```text
data/downloads.json
```

结构：

```json
{
  "version": 1,
  "records": [
    {
      "id": "download-001",
      "filename": "report.xlsx",
      "path": "data/Downloads/report.xlsx",
      "url": "https://...",
      "source_site": "chatgpt",
      "status": "completed",
      "bytes": 123456,
      "created_at": "2026-05-30T12:00:00+08:00",
      "completed_at": "2026-05-30T12:00:03+08:00",
      "error": null
    }
  ]
}
```

状态枚举：

- `started`
- `completed`
- `failed`
- `cancelled`
- `missing`

### 8.4 下载动作

每条记录支持：

- 打开文件
- 打开所在文件夹
- 复制路径
- 删除记录
- 重新下载

删除记录默认不删除文件。后续可以加“同时删除本地文件”确认项。

### 8.5 搜索

搜索范围：

- 文件名
- 来源站点
- 保存路径
- 下载 URL

不搜索文件内容，避免性能问题。

### 8.6 下载设置

下载设置参考截图：

```text
下载内容保存位置：
○ 使用上次下载目录
● 固定目录：data\Downloads       [更改...]
```

推荐默认：

```text
程序目录\data\Downloads
```

如果程序目录不可写，自动回退：

```text
用户下载目录\EasyGPT
```

设置保存到 `settings.toml`：

```toml
[downloads]
save_mode = "fixed"
fixed_dir = "data/Downloads"
last_dir = ""
ask_each_time = false
max_records = 500
```

### 8.7 新建下载

“新建下载”提供 URL 输入框：

```text
下载地址：[________________]
保存为：  [自动识别文件名]
[开始下载] [取消]
```

首版可以只支持普通 HTTP/HTTPS 下载，不支持需要登录 Cookie 的复杂页面资源。

## 9. 导出能力

### 9.1 Markdown 导出

适用场景：ChatGPT 对话结构化保存。

要求：

- 提取用户/助手消息
- 保留代码块
- 保留来源 URL
- 文件保存到下载中心配置目录
- 写入下载记录

### 9.2 PDF 导出

两种模式：

1. WebView2 `PrintToPdf`
2. 长截图拼接为 PDF

优先级：

- 当前页面视觉还原：长截图 PDF
- 普通网页打印：PrintToPdf
- 失败时回退 Markdown 导出

### 9.3 长截图导出

实现思路：

- 通过 WebView2 或 CDP 获取页面高度
- 分段截图
- 合并为 PDF
- 限制最大页面高度，避免内存爆炸

保护限制：

- 单次最大截图高度：30000px
- 单页图片最大内存：100MB
- 超限时提示用户分段导出

## 10. 性能与内存

### 10.1 WebView 生命周期

内存优化按钮执行：

- 保留 ChatGPT
- 保留当前页面
- 释放其他后台 WebView
- 请求 WebView GC
- Windows 下调用低内存策略

### 10.2 UI 节流

以下 UI 更新必须节流：

- 下载进度：500ms
- 节点测速结果：批量刷新
- 日志尾部：最多 1s 一次
- 延时检测：自动 60s，手动立即

### 10.3 数据上限

下载记录默认最多 500 条。

日志显示最多 300 行。

节点测速缓存最多保留当前订阅所有节点最近一次结果。

### 10.4 卡顿风险控制

禁止在 UI 线程执行：

- 订阅下载
- 节点测速
- 文件写入大内容
- PDF/截图生成
- mihomo 控制器请求

所有这些都必须在线程池或后台线程执行，通过 `UserEvent` 回到 UI。

## 11. 诊断报告

新增“导出诊断”按钮。

生成：

```text
data/diagnostics/easygpt-diagnostic-YYYYMMDD-HHMMSS.zip
```

包含：

- app version
- settings.toml，隐藏订阅 token
- mihomo.log 最近 300 行
- config.yaml，隐藏 server/password/token
- 当前端口状态
- 当前代理模式
- 当前订阅名
- 当前策略组/节点
- WebView2 runtime 版本
- 最近崩溃日志摘要

诊断报告不得包含 Cookie、LocalStorage、完整 WebView2Profile。

## 12. 配置迁移

新增：

- 导出便携数据包
- 导入便携数据包

导出内容：

```text
settings.toml
downloads.json
clash/subscriptions
clash/subscription cache
WebView2Profile
```

导入时：

- 关闭所有 WebView
- 备份当前 data
- 解压新 data
- 重启应用

提示用户：登录态可能因为站点安全策略要求重新验证。

## 13. 自动更新与打包

### 13.1 本地打包

继续支持：

```powershell
scripts/package-portable.ps1
scripts/package-installer.ps1
```

### 13.2 GitHub Actions

新增 Windows 构建流程：

- checkout
- setup Rust MSVC
- cache cargo
- build release
- ensure mihomo
- package portable zip
- package installer exe
- upload artifacts
- release 时上传到 GitHub Releases

### 13.3 更新检查

首版只做手动检查更新：

- 当前版本
- 最新版本
- 下载链接

不做静默自动安装。

## 14. 设置中心结构

设置中心分组：

1. 代理
2. 订阅
3. 节点
4. 下载
5. 性能
6. 诊断
7. 关于

设计原则：

- 居中弹窗
- 底部固定保存/关闭按钮
- 保存后明确提示是否需要重启
- 不在左侧显示易误触按钮

## 15. 数据文件总览

```text
data/
  settings.toml
  downloads.json
  Downloads/
  diagnostics/
  WebView2Profile/
  clash/
    config.yaml
    logs/mihomo.log
    subscriptions/
      sub-xxx.yaml
```

## 16. 测试计划

### 16.1 单元测试

覆盖：

- settings 序列化/反序列化
- downloads.json 读写
- 下载路径解析和重名处理
- 订阅列表增删改查
- 节点选择保存/恢复
- 启动状态机状态转换
- 超时错误生成
- WebView bounds 计算
- shell IPC 命令解析

### 16.2 集成测试

覆盖：

- 使用测试订阅启动 mihomo
- Controller API 可访问
- 选择节点后 settings 更新
- 下载文件写入配置目录
- 下载中心读取历史记录
- 诊断报告脱敏

### 16.3 手动验收

必须验证：

- 首次启动能看到状态进度
- 代理失败 15 秒内出现错误页
- 点击重试代理可恢复
- 外部 Clash/cclash 开着时，本应用仍使用自己的端口
- ChatGPT 能打开
- 四个顶部页面可切换
- 下载文件能出现在下载中心
- 修改保存路径后新下载写到新路径
- 重启后下载记录还在
- 点击打开文件和打开目录有效
- 清理内存后 ChatGPT 不丢失
- 关闭应用后 bundled mihomo 不残留

## 17. 验收标准

### 17.1 稳定性

- 连续启动 10 次不闪退
- 代理启动失败不无限等待
- 关闭应用后无 bundled mihomo 残留

### 17.2 性能

- 冷启动到可见窗口小于 3 秒
- 代理正常时 15 秒内打开 ChatGPT
- 常规使用内存稳定，不随切换页面无限增长
- 下载中心 500 条记录下打开不卡顿

### 17.3 功能

- 多订阅不会相互覆盖
- 节点选择能保存并下次恢复
- 下载保存路径可设置
- 下载历史可搜索
- 诊断报告可生成且脱敏

## 18. 对抗式审阅

### 18.1 风险：功能太多，开发容易失控

问题：本文档覆盖启动、代理、下载、性能、导出、迁移、更新，如果一次性全部开发，回归风险很高。

修正：按版本切片。v0.10 只做启动和代理状态机，v0.11 只做下载中心，v0.12 再做性能。每版都要能独立发布。

### 18.2 风险：下载中心实现成复杂下载器

问题：如果支持断点续传、多线程、登录态下载、任意请求头，会快速变成下载软件。

修正：首版只做 WebView2 原生下载事件和简单 HTTP/HTTPS 新建下载。复杂下载放弃。

### 18.3 风险：便携登录态承诺过度

问题：用户可能以为复制 data 就一定免登录，但 Google/ChatGPT 可能绑定设备或要求验证。

修正：文案写“尽量保留登录态”，不要承诺跨设备 100% 可用。迁移功能要显示风险提示。

### 18.4 风险：内置代理和外部 Clash 冲突

问题：用户开着 cclash，端口可能占用，或者系统代理指向外部 Clash。

修正：InternalClash 模式必须解析可用端口，并把 WebView2 显式指向本应用 mixed port。不得依赖系统代理。

### 18.5 风险：RuntimeReady 丢失导致等待页不跳转

问题：mihomo 已启动，但 WebView 仍停在等待页。

修正：RuntimeReady 后必须对所有已创建内容 WebView 执行站点专属导航脚本；未来创建的 WebView 直接打开真实 URL。等待页增加手动“继续打开”按钮作为兜底。

### 18.6 风险：节点测速导致 UI 卡顿

问题：全量节点测速可能几十个请求同时发起，拖慢 UI。

修正：测速在后台线程执行，并发限制 4，结果批量回传，UI 节流刷新。

### 18.7 风险：下载记录越来越大

问题：长期使用后 `downloads.json` 过大，下载中心打开慢。

修正：默认最多 500 条，超过后清理最旧记录。后续可做归档。

### 18.8 风险：长截图 PDF 内存爆炸

问题：超长 ChatGPT 对话截图可能占用巨大内存。

修正：设置最大高度和最大图片内存，超限提示分段导出。

### 18.9 风险：诊断报告泄露隐私

问题：配置、订阅、日志可能包含 token、server、password。

修正：导出前必须脱敏。不得包含 Cookie、WebView2Profile、LocalStorage。

### 18.10 风险：设置项太多变复杂

问题：设置中心如果塞满代理、下载、性能、诊断，用户会迷路。

修正：分组显示，默认只展开常用项。高级项折叠。

### 18.11 风险：安装目录不可写

问题：安装到 `Program Files` 时，程序目录 `data/Downloads` 可能不可写。

修正：安装器默认使用 `%LOCALAPPDATA%\Programs\ChatGPTWebviewClient`。如果不可写，自动回退用户下载目录。

### 18.12 风险：自动更新引入安全问题

问题：自动下载执行安装包可能被劫持。

修正：首版只检查更新并跳转 GitHub Release。后续再考虑签名校验。

## 19. 推荐开发顺序

1. 启动状态机和代理超时页
2. RuntimeReady 导航可靠性和等待页兜底
3. 下载中心数据模型和持久化
4. 下载中心 UI 和保存路径设置
5. 节点测速限流和缓存
6. WebView 内存释放策略
7. 诊断报告
8. 长截图导出
9. 配置导入导出
10. GitHub Actions 打包和更新检查

## 20. 第一阶段实施拆解

第一阶段建议只做 v0.10 + v0.11 的核心：

### Task A：启动状态机

- 新增 `StartupStage`
- 增加 `UserEvent::StartupProgress`
- 等待页显示当前阶段
- 超时 15 秒显示操作按钮

### Task B：RuntimeReady 修复

- 确保 RuntimeReady 后主动导航等待页
- 等待页增加手动继续按钮
- 增加启动后健康检查

### Task C：下载配置

- 在 `settings.toml` 增加 `[downloads]`
- 实现默认目录、固定目录、上次目录
- 不可写时回退

### Task D：下载记录持久化

- 新增 `data/downloads.json`
- 下载开始/完成/失败都写记录
- 启动时读取历史

### Task E：下载中心 UI

- 顶部下载按钮打开下载中心
- 列表、操作按钮、底部工具栏、搜索框
- 下载设置弹窗

### Task F：验证

- `cargo fmt --check`
- `cargo test`
- `cargo clippy --all-targets -- -D warnings`
- release build
- portable package
- 实机启动和下载验证

## 21. 开发完成定义

一个版本只有满足以下条件才算完成：

1. 代码格式、测试、clippy 全部通过。
2. release build 成功。
3. portable package 成功。
4. 新 EXE 启动观察不少于 30 秒。
5. 对应核心功能完成手动验收。
6. 发现失败时不能只改 UI 文案，必须定位根因。

