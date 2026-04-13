---
name: clone-generate
when_to_use: 用户要求创建一个新的分身，或描述了一个需要分身来完成的需求
allowed_tools: ["file_write", "file_read", "file_list", "shell_exec", "web_fetch", "knowledge_lint", "clone_evaluate"]
version: 2
usage_count: 0
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

信息收集完毕后，按以下结构生成文件：

```
/tmp/<clone-name>/
├── template.json
├── profile.md
├── SOUL.md
├── system_prompt.md
├── MEMORY.md
├── EVOLUTION.md
├── knowledge/
│   └── *.md          # 每个文件使用双层格式 + 完整 frontmatter
├── skills/
│   └── *.md
├── agents/            (可选，复杂分身需要子代理时)
│   └── *.md
└── style/            (可选，如有风格数据)
    └── *.md
```

使用 file_write 工具写入每个文件。

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

### 3. 打包

```bash
cd /tmp && tar czf <clone-name>.agx -C <clone-name> .
```

### 4. 安装

将 .agx 文件 base64 编码后通过 API 安装：

```bash
BASE64_DATA=$(base64 -i /tmp/<clone-name>.agx)
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64_DATA\"}"
```

或者如果需要指定用户：
```bash
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64_DATA\", \"user_id\": \"<user_id>\"}"
```

### 5. 安装后验证

安装成功后，执行健康检查和质量评估：

```bash
# 检查知识库健康状态
curl -s http://localhost:4200/api/clones/<name>/health

# 评估分身质量得分
curl -s http://localhost:4200/api/clones/<name>/evaluate
```

### 6. 确认

安装和验证完成后告诉用户：
- 分身名称和 ID
- 质量评分（来自 evaluate）
- 如有健康问题，列出需要修复的项
- 可以通过 `/api/clones` 查看已安装分身
- 可以通过 `/api/clones/<name>/start` 启动分身
- 分身运行后会自动学习新知识（evolution），自动优化（compile）

## 生成规则

- template.json 的 version 固定为 "1"
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
