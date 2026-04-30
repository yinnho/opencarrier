# .agx 分身包格式规范

## 概述

.agx 是 OpenCarrier 的分身打包格式，本质是 tar.gz 归档文件。
分身 = 13 个组成部分，其中身份相关文件打包进 .agx，运行时数据不包含。

## 目录结构

```
<clone-name>.agx (tar.gz)
├── template.json       # 元数据（必需）
├── profile.md          # 名称、描述、标签（必需）
├── SOUL.md             # 人格定义（可选但推荐）
├── system_prompt.md    # 系统指令（可选但推荐）
├── MEMORY.md           # 知识索引（可选）
├── EVOLUTION.md        # 进化策略（推荐）
├── knowledge/          # 知识文件目录
│   ├── faq.md
│   └── product-info.md
├── skills/             # 技能目录
│   ├── booking.md      # 扁平格式
│   └── search/         # 目录格式
│       ├── SKILL.md
│       └── scripts/
│           └── api.toml
├── agents/             # 子代理目录（可选）
│   ├── code-reviewer.md # 扁平格式
│   └── architect/      # 目录格式
│       ├── AGENT.md
│       └── scripts/
│           └── analyze.toml
└── style/              # 风格样本（可选）
    └── chat-style.md
```

**不包含**: output/、sessions/、history/（运行时数据，不属于分身身份）。

## 各文件详细格式

### template.json

```json
{
  "version": "1",
  "name": "clone-name",
  "description": "一句话描述",
  "author": "作者名",
  "tags": ["tag1", "tag2"],
  "exported_at": "1712736000",
  "knowledge_version": 2,
  "plugins": ["wecom"]
}
```

**`plugins`**（可选）：字符串数组，声明该分身依赖的插件。值对应 `~/.opencarrier/plugins/` 下的目录名（不含 `opencarrier-plugin-` 前缀）。安装时系统会检查依赖的插件是否已加载。如果分身需要使用插件提供的工具，必须声明对应的插件依赖。

### profile.md — YAML frontmatter + Markdown

```yaml
---
name: clone-name
description: 一句话描述
type: training
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

### SOUL.md — 纯 Markdown

定义分身的"灵魂"，包括：
- 身份（我是谁）
- 性格特征
- 工作风格
- 沟通偏好

**绝不包含**: 工作规则、工作流程、参考资料。

### system_prompt.md — 纯 Markdown

分身的核心系统指令。结构：
1. 角色定位
2. 核心能力
3. 工作流程/处理逻辑
4. 行为约束和规则
5. 输出格式要求

**绝不包含**: 人格描述、FAQ 条目、示例代码。

### MEMORY.md — Markdown 知识索引

```markdown
# 知识索引

## 知识
- [退款政策](data/knowledge/refund-policy.md)
- [发货流程](data/knowledge/shipping-process.md)

## 技能
- **handle-refund** — 当用户要求退款时使用
```

此文件安装后会由系统自动维护，不需要手动更新。

### EVOLUTION.md — 进化策略（推荐）

每个分身的学习配置。不同分身有不同进化策略。

```markdown
---
evolution_mode: conservative
max_knowledge_files: 200
knowledge_capacity_mb: 50
auto_compile: true
compile_interval_hours: 24
bloat_stale_days: 30
bloat_delete_days: 60
feedback_to_hub: false
---

## 进化规则
- 客服分身：只提取事实性知识（政策、流程），不提取闲聊
- 技术分身：提取技术方案和最佳实践
- 销售分身：提取话术和客户异议处理经验
```

**evolution_mode 取值**:
- `conservative` — 只提取明确的知识（默认）
- `aggressive` — 积极提取所有可能的知识
- `disabled` — 关闭自动学习

**缺失时**: 使用默认配置（conservative 模式，200 文件上限，50MB 容量）。

### knowledge/*.md — 知识文件（双层格式）

每个知识文件使用**双层格式**：上半部分是可编译的真相（可被系统修改），下半部分是追加式时间线。

```markdown
---
name: 退款政策
source: manual
type: knowledge
description: 公司退款政策详细说明
tags: [退款, 售后]
confidence: EXTRACTED
status: active
---

购买后7天内可以无条件退款。退款将在3个工作日内原路返回。
跨境电商订单不支持无理由退款，需提供商品问题证明。

---

- 2026-04-12: 从产品文档导入
```

**Frontmatter 必填字段**:
| 字段 | 说明 | 取值 |
|------|------|------|
| `name` | 知识标题 | 自由文本 |
| `source` | 来源 | `manual`（手动创建）/ `evolution`（对话提取）/ `import`（导入）/ `conversation` |
| `description` | 简短描述 | 一句话概括内容（compile 可自动生成） |
| `confidence` | 置信度 | `EXTRACTED`（直接提取）/ `INFERRED`（推断得出）/ `AMBIGUOUS`（待确认） |
| `tags` | 标签 | YAML 数组 |
| `status` | 状态 | `active` / `stale` / `expired` |

**双层分隔符**: 第二个 `---`（空行包围）分隔编译层和时间线。时间线使用 `- YYYY-MM-DD: 描述` 格式追加，不修改上方内容。

**置信度说明**:
- `EXTRACTED` — 直接从对话/文档中提取的事实，可信度最高
- `INFERRED` — 由系统从上下文推断，需要验证升级
- `AMBIGUOUS` — 存在歧义，需要人工确认（health check 会标记警告）

**生命周期**:
- 对话后自动进化写入（confidence: INFERRED）
- 用户验证后升级为 EXTRACTED
- compile 时自动生成缺失的 description/tags
- 30 天未引用标记 stale，60 天删除

### style/*.md — 风格样本（可选）

从聊天记录提取的说话风格。不是人格定义，是真实对话中的风格模式。

```markdown
---
name: 微信风格样本
source: import
type: style
---

## 常用句式
- "好的呢~"
- "马上帮您看看"
- "亲~您的订单号是多少呀"

## 情绪模式
- 遇到投诉先安抚
- 结束语固定："还有什么可以帮您的吗？"
```

**创建方式**: 用户导入聊天记录时由 LLM 分析提取，或手动编写。

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

### agents/*.md — 子代理定义（扁平格式）

子代理是可派出去干活的专门角色。每个 agent 有自己的指令、工具白名单和模型选择。

**与 skills 的区别**：Skills = "做什么"（操作手册），Agents = "谁来做"（执行实体）。

```yaml
---
name: code-reviewer
description: 专门做代码审查的子代理
tools: ["Glob", "Grep", "Read", "Bash"]
model: sonnet
color: red
---

# Code Reviewer

你是代码审查专家。分析代码质量、安全漏洞、性能问题。
按以下维度审查：正确性、安全性、性能、可维护性。
```

**Frontmatter 字段**:
| 字段 | 类型 | 说明 |
|------|------|------|
| `name` (必需) | string | 子代理名称 |
| `description` (必需) | string | 一句话描述 |
| `tools` | string[] | 允许使用的工具白名单 |
| `model` | string | 模型选择：sonnet / haiku / opus（默认 sonnet） |
| `color` | string | 可选，UI 标识色 |

### agents/<name>/AGENT.md + scripts/*.toml — 子代理定义（目录格式）

AGENT.md 格式同上。scripts/*.toml 定义子代理专用的 API 调用，格式与 skill scripts 相同。

**使用场景**:
- 复杂任务需要并行处理（同时审查安全 + 性能 + 风格）
- 子任务需要不同的模型（主代理用 opus，子代理用 sonnet）
- 子任务需要隔离的工具权限（审查员只读不能写）
- Claude Code 插件中的 agents/ 可直接映射到此目录

## 打包注意事项

1. 文件路径可以用 `./` 前缀或不用，加载器自动处理
2. macOS 的 `._*` Apple Double 文件会被自动过滤
3. 知识文件大小建议不超过 100KB
4. 安全扫描会检查：注入关键词、超大文件、非 HTTPS URL
5. 编码统一使用 UTF-8
6. EVOLUTION.md 推荐包含，缺失时使用系统默认配置
7. 知识文件的 confidence 字段影响质量评分：EXTRACTED 占比越高分数越高

## 安装后自动流程

安装完成后，系统会自动：
1. 创建运行时目录（output/、sessions/、history/）
2. 建立 `history/versions.jsonl` 版本追踪
3. 首次 compile 生成缺失的 description/tags
4. 建立 `.lifecycle/manifest.json` 增量编译清单

建议安装后手动执行：
- `knowledge_lint` — 检查知识库健康状态
- `clone_evaluate` — 评估分身质量得分
