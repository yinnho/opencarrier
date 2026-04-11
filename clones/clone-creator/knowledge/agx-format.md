# .agx 分身包格式规范

## 概述

.agx 是 OpenClone/OpenCarrier 的分身打包格式，本质是 tar.gz 归档文件。

## 目录结构

```
<clone-name>.agx (tar.gz)
├── template.json       # 元数据（必需）
├── profile.md          # 名称、描述、标签（必需）
├── SOUL.md             # 人格定义（可选但推荐）
├── system_prompt.md    # 系统指令（可选但推荐）
├── MEMORY.md           # 知识索引（可选）
├── knowledge/          # 知识文件目录
│   ├── faq.md
│   └── product-info.md
└── skills/             # 技能目录
    ├── booking.md      # 扁平格式
    └── search/         # 目录格式
        ├── SKILL.md
        └── scripts/
            └── api.toml
```

## 各文件详细格式

### template.json

```json
{
  "version": "1",                    // 格式版本，固定为 "1"
  "name": "clone-name",              // 分身名称
  "description": "一句话描述",        // 简短描述
  "author": "作者名",                 // 作者
  "tags": ["tag1", "tag2"],          // 分类标签
  "exported_at": "1712736000",       // 导出时间 Unix 时间戳字符串
  "knowledge_version": 2             // knowledge/ 目录下文件数量
}
```

### profile.md — YAML frontmatter + Markdown

```yaml
---
name: clone-name
description: 一句话描述
type: training          # "training" 或 "serving"
tags: ["tag1"]
---
# Clone Name

正文描述...
```

支持的 frontmatter 字段：
- `name` (必需) — 分身名称
- `description` — 描述
- `type` — "training"（训练模式）或 "serving"（服务模式）
- `tags` — 标签数组
- `source_template` — 来源模板（fork 时自动注入）
- `source_author` — 来源作者
- `forked_at` — fork 时间戳
- `status` — 知识状态: "active" / "expired" / "compressed"

### SOUL.md — 纯 Markdown

定义分身的"灵魂"，包括：
- 身份（我是谁）
- 性格特征
- 工作风格
- 沟通偏好

### system_prompt.md — 纯 Markdown

分身的核心系统指令，LLM 每次对话都会看到。结构：
1. 角色定位
2. 核心能力
3. 工作流程/处理逻辑
4. 行为约束和规则
5. 输出格式要求

### MEMORY.md — Markdown 知识索引

```markdown
# 知识索引

## 分类1
- filename — 简短描述

## 分类2
- filename — 简短描述
```

### knowledge/*.md — 知识文件

每个 Markdown 文件包含一个主题的详细知识。文件名用短横线分隔，如 `product-faq.md`。

### skills/*.md — 技能定义（扁平格式）

```yaml
---
name: skill-name
when_to_use: 触发条件描述
allowed_tools: ["tool1", "tool2"]
version: 1
usage_count: 0
---
# Skill Name

详细的执行步骤和使用说明...
```

### skills/<name>/SKILL.md + scripts/*.toml — 技能定义（目录格式）

SKILL.md 格式同上。scripts/*.toml 定义 HTTP API 调用：

```toml
name = "api_call_name"
description = "API 调用描述"

[[parameters]]
name = "param1"
required = true
description = "参数描述"

[request]
url = "https://api.example.com/endpoint"
method = "GET"

[request.query]
param1 = "{{param1}}"
```

## 打包注意事项

1. 文件路径可以用 `./` 前缀（如 `./profile.md`）或不用（如 `profile.md`），加载器会自动处理
2. macOS 的 `._*` Apple Double 文件会被自动过滤
3. 知识文件大小建议不超过 100KB
4. 安全扫描会检查：注入关键词、超大文件、非 HTTPS URL
5. 编码统一使用 UTF-8
