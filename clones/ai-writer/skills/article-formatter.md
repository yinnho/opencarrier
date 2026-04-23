---
name: article-formatter
when_to_use: 当文章写完需要排版为公众号 HTML，或用户说"排版"、"转HTML"、"公众号格式"时
allowed_tools: []
version: 1
usage_count: 0
---

# Article Formatter

将 Markdown 文章转换为微信公众号兼容的内联样式 HTML。

## Process

### 1. 读取 Markdown

获取待转换的 Markdown 文本。

### 2. 逐元素转换

按照 knowledge/wechat-html-spec.md 中的规范转换：

| Markdown | HTML |
|----------|------|
| `## 标题` | `<section>` + 左边框蓝色标题样式 |
| `**加粗**` | `<strong style="...">` |
| `` `代码` `` | `<code style="...">` |
| 代码块 | `<section>` 深色背景 + 等宽字体 |
| `> 引用` | `<section>` 左边框灰色斜体 |
| `![img](url)` | `<section>` 居中 + max-width:100% |
| `- 列表` | `<section>` + `•` 前缀 |
| `---` | `<section>` 上边框分隔线 |
| `[text](url)` | `<a>` 蓝色链接 |
| 普通段落 | `<section>` + 2em 缩进 + 1.8 行高 |

### 3. 后处理

- 所有 `<div>` 替换为 `<section>`
- 压缩 HTML：标签间不留多余换行/空格
- 确认没有 CSS class、没有 `<style>` 标签
- 确认所有样式都是内联的

### 4. 输出

输出完整的 HTML 字符串，可直接用于 `mcp_wechat_oa_create_draft` 的 `content` 参数。

## Important Principles

- 严格遵循公众号 HTML 规范，不使用不支持的标签
- 不使用 `<h1>`（公众号有自己的标题系统）
- 图片如果使用外部 URL，确保 URL 可公开访问
- 代码块必须用深色背景的 `<section>` 包裹，不用 `<pre>` 外层
