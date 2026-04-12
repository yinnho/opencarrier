# 分身完整结构定义

> **核心原则**: 分身 = 插拔式人格，载体 = 身体（OS）。分身定义你是谁，载体提供能力。
> **一户一备**: 每个用户自部署一个 opencarrier 实例，从 Hub 下载分身运行。

---

## 分身 = 12 个组成部分

分身不是一个文件，而是一个**目录**。每个部分有严格的责任边界，绝不混淆。

```
~/.opencarrier/workspaces/<name>/
├── profile.md              # 1.  分身档案（身份卡片）
├── SOUL.md                 # 2.  人格/灵魂（你是谁）
├── system_prompt.md        # 3.  行为指令/大脑（你怎么做事）
├── MEMORY.md               # 4.  知识索引（系统自动维护，永远加载）
├── data/knowledge/         # 5.  参考资料/知识库（按需加载）
├── skills/                 # 6.  能力模块（两步激活）
├── style/                  # 7.  风格样本（从聊天记录提取的说话风格）
├── output/                 # 8.  工作产物（分身生成的文件）
├── sessions/               # 9.  对话历史（JSONL 格式）
├── data/orchestrator.md    # 10. 编排对话记录（训练模式日志）
├── history/                # 11. 版本追踪
│   └── versions.jsonl      #     知识变更日志（创建/修改/验证/回滚）
└── EVOLUTION.md            # 12. 进化策略（每个分身的学习配置）
```

---

## 每个部分的详细定义

### 1. profile.md — 分身档案

**定义**: 身份卡片。名称、描述、来源、标签。

```yaml
---
name: customer-service-bot
description: 某某公司的客服分身
type: serving          # serving（生产）或 training（训练中）
tags: [客服, 电商, 售后]
source_template: customer-service-v2  # fork 来源（Hub 模版名）
source_author: yinnho
forked_at: 2026-04-01
---
```

**注入时机**: 不直接注入 system prompt。分身不需要知道自己的档案。
**用途**: Hub 发布时读取元数据、API `/api/agents/:id` 返回给前端展示。

---

### 2. SOUL.md — 人格/灵魂

**定义**: 你是谁。性格、语气、说话风格、情感模式、行为边界。

**包含**:
- 性格特点（热情、专业、幽默、严谨…）
- 语气（正式？随意？温暖？）
- 常用表达和情绪模式
- 边界（该说什么、不该说什么）

**绝不包含**: 工作规则（→ system_prompt.md）、工作流程（→ system_prompt.md）、参考资料（→ knowledge/）

**示例**:
```markdown
你是小薇，一个活泼热情的客服小姐姐。

## 性格
- 热情开朗，喜欢用感叹号和表情
- 耐心细致，从不催促客户
- 遇到投诉时先安抚情绪再解决问题

## 语气
- 口语化，不用书面语
- 常用"亲~"、"好的呢~"、"马上帮您看看"
- 结束语固定："还有什么可以帮您的吗？"
```

**注入优先级**: 最高。始终注入 system prompt 开头，后跟引导语：
> "体现以上人格和语气。避免生硬通用的回复，按上面定义的方式说话。"

---

### 3. system_prompt.md — 行为指令/大脑

**定义**: 你怎么做事。能力、规则、工作流程、输出格式。

**包含**:
- 能力（我能做什么）
- 规则（我必须遵守什么）
- 工作方式（我怎么做事情）
- 输出格式（我输出什么）

**绝不包含**: 人格描述（→ SOUL.md）、FAQ 条目（→ knowledge/）、示例代码（→ knowledge/）

**注入优先级**: SOUL.md 之后，始终注入。

---

### 4. MEMORY.md — 知识索引

**定义**: 系统自动维护的文件目录。扫描 knowledge/、style/、skills/ 自动生成。

**结构**:
```markdown
# 知识索引

> 此文件由系统自动维护，不要手动编辑。

## 知识
- [退款政策](data/knowledge/refund-policy.md)
- [发货流程](data/knowledge/shipping-process.md)

## 风格
- [微信风格样本](data/style/wechat-style.md)

## 技能
- **handle-refund** — 当用户要求退款时使用
- **query-logistics** — 当用户查物流时使用

## 知识缺口
- [待补充] 国际物流的时效标准
```

**注入优先级**: 始终注入。让 LLM 知道有哪些文件可以查阅。
**进化系统写入**: 新知识写入后自动重建此索引。

---

### 5. knowledge/ — 参考资料/知识库

**定义**: 按需加载的参考材料。每个文件是独立的 Markdown + YAML frontmatter。

```markdown
---
name: 退款政策
source: evolution
type: knowledge
description: 公司退款政策详细说明
tags: [退款, 售后]
status: active
---

购买后7天内可以无条件退款...
```

**加载方式**: LLM 通过 `knowledge_list` 工具发现文件，`knowledge_read` 工具读取内容。
**不预加载**: 避免占用上下文预算。50 个知识文件全加载 = 50-100KB。

**生命周期**:
- 对话后自动进化写入（evolution）
- 重复知识合并（compile）
- 30 天未引用标记过期（bloat）
- 60 天未引用删除（bloat）

---

### 6. skills/ — 能力模块

**定义**: 结构化操作能力。每个 skill = 什么时候用 + 用什么工具 + 怎么做。

**两步激活机制**:

**Step 1 — 目录（始终注入，短）**:
```
1. **handle-refund** — 当用户要求退款时
2. **query-logistics** — 当用户查物流时
```

**Step 2 — 完整 prompt（按需注入）**:
当 LLM 匹配到 skill 的 `when_to_use` 时，注入完整 skill body + allowed_tools。
这发生在 agent loop 中间，不是预加载。

**Skill 文件格式**:
```markdown
---
name: handle-refund
when_to_use: 当用户要求退款或表达退款意图时
allowed_tools: [file_read, file_write, knowledge_read]
version: 1
usage_count: 0
---

## 退款处理流程
1. 确认订单号
2. 查询订单状态
3. 检查退款条件（7天内）
4. 执行退款并通知客户
```

**Skill 脚本** (`skills/<name>/scripts/*.toml`):
```toml
[script.refund-api]
description = "调用退款 API"
method = "POST"
url = "https://api.example.com/refund"
headers = { Authorization = "Bearer {{api_key}}" }
body = '''{"order_id": "{{order_id}}", "reason": "{{reason}}"}'''
```

**生命周期**: 从进化中诞生（3+次相似请求）→ compile 优化 → 30天不用过期 → 60天删除。

---

### 7. style/ — 风格样本

**定义**: 从聊天记录中提取的说话风格样本。不是人格定义，是**真实对话中的风格模式**。

**内容**:
- 常用句式和口头禅
- 情绪表达方式
- 话题偏好
- 回复长度和节奏

**创建方式**: `openclone import wechat-export.json --sender "小薇"` 时，LLM 分析该发送者的所有消息，提取风格特征写入 `style/wechat-style.md`。

**注入方式**: 与 SOUL.md 配合使用。SOUL.md 定义理想人格，style/ 提供真实风格参考。

---

### 8. output/ — 工作产物

**定义**: 分身在对话中生成的文件。代码、文档、图片等。

**自动保存**: agent loop 检测到代码块（\`\`\`html/css/js 等）时自动保存到 output/。

---

### 9. sessions/ — 对话历史

**定义**: JSONL 格式的对话记录。每次对话一个文件。

**用途**: 进化引擎分析历史对话提取知识、训练模式回放。

---

### 10. orchestrator.md — 编排对话记录

**定义**: 训练模式（clone-creator）的对话日志。记录分身训练过程中的所有交互。

**用途**: 追溯分身是怎么被训练出来的，支持回滚到训练中间状态。

---

### 11. history/versions.jsonl — 版本追踪

**定义**: 知识文件变更日志。每行一条 JSON 记录。

```json
{"timestamp":"2026-04-12T10:30:00Z","action":"create","file":"refund-policy.md","before":null,"after":"...","source":"evolution","verified":false}
```

**操作**: `verify`（验证）、`rollback`（回滚）、`log`（查看历史）。
**来源**: evolution（自动）、user（手动）、verify（验证升级）。

---

### 12. EVOLUTION.md — 进化策略

**定义**: 每个分身的学习配置。不同分身有不同进化策略。

```markdown
---
evolution_mode: conservative    # conservative / aggressive / disabled
max_knowledge_files: 200
knowledge_capacity_mb: 50
auto_compile: true
compile_interval_hours: 24
bloat_stale_days: 30
bloat_delete_days: 60
feedback_to_hub: false          # 是否匿名反馈知识到 Hub
---

## 进化规则
- 客服分身：只提取事实性知识（政策、流程），不提取闲聊
- 技术分身：提取技术方案和最佳实践
- 销售分身：提取话术和客户异议处理经验
```

**缺失时**: 使用默认配置（conservative 模式）。

---

## System Prompt 组装顺序

每次对话时，系统按以下顺序动态组装 system prompt：

```
1. SOUL.md                    ← 人格（最高优先级）
2. 引导语                     ← "体现以上人格和语气..."
3. system_prompt.md           ← 行为指令
4. Skill 目录                 ← 所有 skill 的 name + when_to_use（始终注入，短）
5. [激活的 Skill 完整 prompt]  ← 按需注入（LLM 匹配 when_to_use 时）
6. MEMORY.md                  ← 知识索引（文件目录）
7. [选中的 knowledge 文件]     ← LLM 通过工具按需读取
8. [style/ 风格参考]          ← 配合 SOUL.md 使用
9. 系统段                     ← 当前日期、可用工具、安全规则、频道信息
```

**关键**: 动态组装，每次对话都从文件读取。编辑文件后立即生效，不需要重启。

---

## .agx 包格式

分身在 Hub 上发布和下载时打包为 `.agx` 文件（tar.gz）：

```
clone.agx (tar.gz)
├── template.json       # 模版元数据（版本、名称、描述、作者、标签）
├── profile.md          # 分身档案
├── SOUL.md             # 人格
├── system_prompt.md    # 行为指令
├── MEMORY.md           # 知识索引
├── knowledge/          # 所有知识文件
├── skills/             # 所有技能文件 + 脚本
└── style/              # 风格样本（如有）
```

**不包含**: output/、sessions/、history/（这些是运行时数据，不属于分身身份）。

---

## 责任边界总结

| 文件 | 定义 | 包含 | 排除 |
|------|------|------|------|
| SOUL.md | 你是谁 | 性格、语气、边界 | 工作规则、参考资料 |
| system_prompt.md | 你怎么做事 | 能力、规则、流程 | 人格、FAQ、代码 |
| knowledge/ | 你知道什么 | 事实、FAQ、文档 | 行为规则、人格 |
| skills/ | 你能做什么 | 触发条件+工具+步骤 | 知识事实 |
| style/ | 你怎么说话 | 真实对话风格模式 | 规则、知识 |
| EVOLUTION.md | 你怎么学习 | 进化策略、容量限制 | 人格、知识 |

---

## opencarrier 实现状态

| 部分 | 状态 | 说明 |
|------|------|------|
| profile.md | ✅ 已创建 | converter.rs 写入，未注入 prompt |
| SOUL.md | ✅ 动态组装 | prompt_builder.rs 读取，2000字上限 |
| system_prompt.md | ✅ 动态组装 | prompt_builder.rs 读取，4000字上限 |
| MEMORY.md | ✅ 动态组装 | prompt_builder.rs 读取，1000字上限 |
| knowledge/ | ⚠️ 工具访问 | knowledge_list + knowledge_read 工具，不预加载 |
| skills/ | ✅ 目录+完整 prompt | read_skills_catalog() + read_workspace_skills_prompts() |
| style/ | ✅ 动态组装 | read_style_samples() 读取并注入 prompt_builder |
| output/ | ✅ 目录存在 | ensure_workspace() 创建 |
| sessions/ | ✅ JSONL | kernel.rs 写入 |
| orchestrator.md | ❌ 未实现 | 无编排训练模式 |
| history/versions.jsonl | ✅ Phase 10 P0 | opencarrier-lifecycle crate |
| EVOLUTION.md | ✅ 进化策略 | evolution_config.rs 解析，kernel hook 检查 |
| 进化引擎 | ✅ Phase 10 P0 | 对话后自动提取知识 |

---

## 下一步计划

**P1 — 补全缺失部分**:
- style/ 风格样本注入到 prompt
- skills 两步激活（agent loop 中按需注入完整 prompt）
- profile.md 注入到 prompt（或作为 API 返回）
- EVOLUTION.md 进化策略配置文件

**P2 — 生态**:
- 分身质量评估（evaluate）
- 反馈回流 Hub（feedback）
- 编排训练模式（orchestrator）
