---
name: WeChat HTML Spec
source: manual
type: knowledge
description: 微信公众号 HTML 排版规范 — 将 Markdown 转为公众号兼容的内联样式 HTML
tags: [wechat, html, formatting]
confidence: EXTRACTED
status: active
---

# 公众号 HTML 排版规范

## 核心约束

微信公众号编辑器**不支持**：
- CSS class 选择器（所有样式必须内联）
- `<div>` 标签（用 `<section>` 替代）
- `<style>` 标签
- JavaScript
- 外部字体
- `<h1>` 标签（标题用自己的排版）

## 转换规则

### 段落
```html
<section style="margin-bottom: 1em; text-indent: 2em; line-height: 1.8; font-size: 16px; color: #333;">
  段落文字...
</section>
```

### 标题（H2 级别）
```html
<section style="margin-top: 1.5em; margin-bottom: 0.8em;">
  <span style="font-size: 20px; font-weight: bold; color: #1a1a1a; border-left: 4px solid #1890ff; padding-left: 12px;">
    标题文字
  </span>
</section>
```

### 加粗
```html
<strong style="color: #1a1a1a;">加粗文字</strong>
```

### 行内代码
```html
<code style="background: #f5f5f5; padding: 2px 6px; border-radius: 3px; font-family: monospace; font-size: 14px; color: #c7254e;">code</code>
```

### 代码块
```html
<section style="background: #2d2d2d; border-radius: 6px; padding: 16px; margin: 1em 0; overflow-x: auto;">
  <pre style="margin: 0; white-space: pre-wrap; word-wrap: break-word;">
    <code style="color: #f8f8f2; font-family: 'Courier New', monospace; font-size: 14px; line-height: 1.6;">代码内容</code>
  </pre>
</section>
```

### 引用
```html
<section style="border-left: 4px solid #ddd; padding-left: 16px; margin: 1em 0; color: #666; font-style: italic;">
  引用文字...
</section>
```

### 图片
```html
<section style="text-align: center; margin: 1em 0;">
  <img src="图片URL" style="max-width: 100%; height: auto; border-radius: 4px;" />
</section>
```

### 列表
```html
<section style="margin: 1em 0; padding-left: 2em;">
  <section style="margin-bottom: 0.5em; line-height: 1.8;">• 列表项 1</section>
  <section style="margin-bottom: 0.5em; line-height: 1.8;">• 列表项 2</section>
</section>
```

### 分隔线
```html
<section style="border-top: 1px solid #eee; margin: 2em 0;"></section>
```

### 链接
```html
<a href="URL" style="color: #1890ff; text-decoration: none;">链接文字</a>
```

## 排版原则

1. **div→section**: 所有 `<div>` 替换为 `<section>`
2. **压缩空格**: 标签间不留换行（公众号渲染器会制造"幽灵间隙"）
3. **首行缩进**: 正文段落 `text-indent: 2em`（标题、代码块、列表不缩进）
4. **行高**: 统一 `line-height: 1.8`
5. **字号**: 正文 16px，小标题 20px，代码 14px
6. **配色**: 正文 #333，标题 #1a1a1a，链接 #1890ff，代码背景 #f5f5f5

---

- 2026-04-23: 手动创建，综合 AIWriteX 和 wechat_article_skills 的排版方案
