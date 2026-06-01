# 顶部多页面切换器原型实施计划

> **给 Claude/Codex：** 按任务逐项实现和验证。

**目标：** 构建一个静态 HTML 原型，演示 ChatGPT、Gemini、NotebookLM 和 Google AI Studio 的顶部切换与页面状态保留。

**架构：** 原型放在 `docs/prototypes`。每个站点对应一个 DOM 面板，切换时只隐藏/显示，不重新创建，用来模拟未来 Rust/Wry 多 WebView 的行为。

**技术栈：** HTML、CSS、原生 JavaScript。

---

## 任务 1：创建静态原型

**文件：**

- 新增：`docs/prototypes/top-multipage-switcher-demo.html`

**步骤：**

1. 创建顶部应用栏、四个站点按钮、导航按钮和主内容区域。
2. 添加居中设置弹窗，用于模拟未来设置面板。
3. 为每个站点创建一个持久 DOM 面板。
4. 使用 JavaScript 跟踪 `activeSite`、`startedSites` 和页面笔记内容。
5. 手动打开 HTML，切换四个页面，确认内容不会丢失。
6. 截图检查顶部栏、页面区域和弹窗是否重叠。

## 验证

- 本地浏览器打开原型文件。
- 切换四个页面。
- 在某个页面输入内容，切走再切回，确认仍然存在。
- 使用浏览器截图检查布局。
