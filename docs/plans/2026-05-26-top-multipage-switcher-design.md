# 顶部多页面切换器设计

## 目标

在保持轻量单窗口客户端的前提下，增加 ChatGPT、Gemini、NotebookLM 和 Google AI Studio 的顶部切换入口。

## 推荐交互

采用浏览器式顶部栏。左侧展示四个固定页面按钮；点击页面后，如果页面尚未启动，则懒加载对应 WebView，然后切换到该页面。已经启动的页面隐藏但不销毁，从而保留登录态、页面状态和临时编辑内容。

右侧保留后退、前进、刷新等常用导航按钮。代理设置继续使用一个居中弹窗，避免出现多个设置入口造成误点。

## 实现形态

原型使用静态 HTML 模拟多个页面。Rust/Wry 实现中使用一个顶部 shell WebView 加多个内容 WebView。所有内容 WebView 共享同一个用户数据目录，保证登录态尽量持久。

站点映射：

- ChatGPT：`https://chatgpt.com`
- Gemini：`https://gemini.google.com`
- NotebookLM：`https://notebooklm.google.com`
- Google AI Studio：`https://aistudio.google.com`

## 持久化

所有 WebView 共享程序数据目录下的 `data/WebView2Profile`。当前活跃页面和已启动页面列表可放入设置文件，方便后续恢复工作区。

## 错误处理

页面加载失败时保留标签入口，并提供刷新或重试。代理状态是全局状态，因为内置 mihomo 影响整个应用的 WebView 环境。

## 原型

静态原型位于：

```text
docs/prototypes/top-multipage-switcher-demo.html
```
