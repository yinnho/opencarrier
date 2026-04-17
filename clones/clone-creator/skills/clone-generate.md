---
name: clone-generate
when_to_use: 用户要求创建一个新的分身，或描述了一个需要分身来完成的需求
allowed_tools: ["file_write", "file_read", "file_list", "clone_install", "clone_export", "web_fetch", "knowledge_lint", "clone_evaluate"]
---

# 分身生成技能

当用户表达了创建分身的意图时，执行以下流程：

## 流程

### 1. 需求收集

通过对话了解以下信息（不必一次问完，可以分步）：

- **分身名称**：英文短横线格式（如 customer-support）
- **用途描述**：一句话说清楚这个分身做什么
- **目标场景**：在什么场景下使用
- **人格特征**：什么性格、什么沟通风格
- **知识领域**：需要了解什么领域的知识
- **技能列表**：需要哪些能力（每个技能有触发条件）
- **进化策略**：保守/积极/关闭（默认保守）

### 2. 文件生成

信息收集完毕后，使用 `clone_install` 工具一次性安装。需要准备以下文件内容：

- **SOUL.md**（必需）：人格定义
- **system_prompt.md**（必需）：行为指令
- **profile.md**（可选）：基本信息
- **MEMORY.md**（可选）：初始知识索引
- **EVOLUTION.md**（推荐）：进化策略
- **knowledge/*.md**：知识文件（路径以 `knowledge/` 开头）
- **skills/*.md**：技能文件（路径以 `skills/` 开头）
- **agents/*.md**：子代理（路径以 `agents/` 开头，可选）
- **style/*.md**：风格文件（路径以 `style/` 开头，可选）

#### 知识文件格式（严格遵守）

每个知识文件必须包含：

```markdown
---
name: <标题>
source: manual
type: knowledge
description: <一句话描述>
tags: [<tag1>]
confidence: EXTRACTED
status: active
---

<知识正文内容>

---

- YYYY-MM-DD: 从用户需求手动创建
```

关键：
- `source: manual` — 因为是 clone-creator 手动创建的
- `confidence: EXTRACTED` — 手动编写的知识是直接提取的事实
- `description` — 一句话概括内容
- 第二个 `---` 分隔符 — 分隔编译层和时间线

#### 技能文件格式

```markdown
---
name: <技能名>
when_to_use: <明确的触发条件>
allowed_tools: ["tool1", "tool2"]
---

<技能 prompt 正文>
```

#### EVOLUTION.md 格式

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
- <根据分身类型定制的知识提取规则>
```

根据分身类型调整：
- 客服分身：`conservative`，只提取事实性知识
- 技术分身：`conservative`，提取技术方案和最佳实践
- 销售分身：`conservative`，提取话术和异议处理
- 研究/创意分身：`aggressive`，广泛提取相关知识

### 3. 安装

使用 `clone_install` 工具一次性完成打包和安装：

```json
{
  "name": "<clone-name>",
  "files": {
    "SOUL.md": "<人格内容>",
    "system_prompt.md": "<系统指令内容>",
    "profile.md": "<基本信息>",
    "EVOLUTION.md": "<进化策略>",
    "knowledge/faq.md": "<FAQ 知识内容>",
    "skills/answer.md": "<技能定义内容>"
  }
}
```

系统会自动完成：
1. 打包为 .agx 格式
2. 创建工作区
3. 安装所有文件
4. 启动分身 agent

**不需要 `shell_exec`，不需要 `tar`，不需要 `curl`。**

### 4. 安装后验证

安装成功后，执行质量评估：

使用 `clone_evaluate` 工具评估分身质量得分。

### 5. 确认

安装和验证完成后告诉用户：
- 分身名称和 ID
- 质量评分（来自 clone_evaluate）
- 如有健康问题，使用 `knowledge_lint` 检查并列出需要修复的项
- 分身运行后会自动学习新知识（evolution），自动优化（compile）

## 导出已有分身

如果用户要求导出已安装的分身，使用 `clone_export` 工具：

```json
{
  "name": "<clone-name>"
}
```

返回 .agx 归档信息。

## 生成规则

- profile.md 必须有 YAML frontmatter
- SOUL.md 用自然语言描述人格，**不包含**工作规则
- system_prompt.md 是最关键的文件，要详细且可操作
- 技能的 when_to_use 必须明确具体
- 知识文件按主题拆分，每个 1000-3000 字为宜
- 所有知识文件使用双层格式（frontmatter + 正文 + `---` + 时间线）
- 手动创建的知识 confidence 设为 EXTRACTED
- 推荐生成 EVOLUTION.md，至少指定 evolution_mode 和进化规则
- 如分身需要并行处理或多角色协作，生成 agents/ 目录
- Skills = "做什么"（操作手册），Agents = "谁来做"（执行实体），两者不要混淆
