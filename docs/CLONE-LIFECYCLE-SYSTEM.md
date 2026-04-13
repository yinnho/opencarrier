# 分身生命周期系统 — 设计文档

> 版本: 4.0 | 日期: 2026-04-12
> 状态: Phase 10-12 已完成

---

## 0. 动机

opencarrier 是分身操作系统。分身不只是"装上就用的静态 agent"——它应该能**学习、成长、自我维护**。

openclone 实现了一套完整的分身训练闭环（数据摄入 → 知识提炼 → 编译优化 → 质量评估 → 运行中进化 → 反馈回流），但这些能力藏在 CLI 命令里，只有人手动调才能用。

**核心洞察：这些能力应该是 opencarrier 的平台级能力，不是某个分身的 skill。**

就像 iOS 的自动更新、健康检查是系统功能而非 App 功能——每个分身装上 opencarrier 就自动拥有学习、进化、自我维护的能力。

---

## 0.1 系统与分身的关系

**系统提供机制，分身提供智能。**

```
系统（opencarrier）                       分身
┌──────────────────────┐               ┌──────────────────────┐
│ 自主功能（分身无感）    │               │ 身份（SOUL.md）        │
│ · 对话后进化           │──自动触发──→  │ 指令（system_prompt）  │
│ · 知识过期清理         │               │ 知识（knowledge/）     │
│ · 版本记录            │               │ 技能（skills/）        │
│                      │               │                      │
│ 系统工具（按需调用）    │               │                      │
│ · knowledge_import   │←─tool_call──│ 分身的 Skill 决定      │
│ · knowledge_compile  │               │ 什么时候用、怎么用     │
│ · knowledge_lint     │               │                      │
│ · clone_evaluate     │               │                      │
│ · feedback_push      │               │                      │
└──────────────────────┘               └──────────────────────┘
```

- **系统 = 身体**（自主神经、代谢、免疫系统）— 进化、清理、版本记录自动运行
- **分身 = 人格**（性格、知识、行为模式）— 四部分文件定义身份
- **系统工具 = 器官**（可用但分身决定何时用）— import、compile、lint、evaluate

**两个例子**：
- 客服分身：从不主动调用 `knowledge_import`，但系统在每次对话后自动进化提取知识
- clone-creator 分身：skill 就是调用系统工具（knowledge_import、knowledge_compile），它是"恰好擅长造分身的分身"，不是系统的一部分

---

## 0.2 已解决的问题（Phase 10-11）

### ~~问题 1：converter.rs 把分身拍扁了~~ ✅ 已修复

converter.rs 不再拼接 system_prompt，只写文件到 workspace。prompt_builder 动态组装。

### ~~问题 2：knowledge_files 字段是死的~~ ✅ 已修复

内核已实现知识注入：read_knowledge_content() 读取 data/knowledge/*.md，注入到 system prompt 的"## 知识库"段。

### ~~问题 3：Skill 激活机制不存在~~ — 待做

clone-creator 的 skill 写在 skills/ 目录里，但运行时只是 manifest 里的一个字符串列表。没有两步激活（先注入目录，再按需注入完整 prompt）。

### ~~问题 4：lifecycle 能力为零~~ ✅ 已修复

opencarrier-lifecycle crate 已创建，包含 8 个模块（evolution、compile、bloat、health、version、evaluate、feedback、parsers），6 个系统工具已注册到 tool_runner。

---

## 0.3 架构修改方向

核心改动：**让内核理解分身的四部分结构**，从静态拼接改为动态组装。

### 现在的流程（错误的）

```
.agx → load → convert（拼成一个大字符串） → agent loop 用死字符串
```

### 修改后的流程（正确的）

```
.agx → load → install 到 workspace（保持四部分文件结构）
                       ↓
agent loop 每次对话时，动态组装 system prompt：
  SOUL.md → system_prompt.md → skill 目录 → MEMORY.md → 按需加载 knowledge
```

### 改动清单

| # | 改什么 | 现在的问题 | 改成什么 | 状态 |
|---|--------|-----------|---------|------|
| 1 | **converter.rs** | 把所有内容拼成一个大 system_prompt 字符串 | 只写文件到 workspace，不拼接 | ✅ 已修复 |
| 2 | **kernel / agent loop** | 用 manifest 里的死 system_prompt 字符串 | 运行时从 workspace 文件动态构建 | ✅ 已修复 |
| 3 | **opencarrier-lifecycle** | 不存在 | 新 crate，所有系统机制操作 workspace 文件 | ✅ 已创建 |
| 4 | **tool_runner** | 没有 knowledge_import 等系统工具 | 注册 6 个系统工具到 tool_runner | ✅ 已注册 |

### 关键原则

1. **Workspace 即分身** — workspace 里的文件（SOUL.md、knowledge/、skills/）就是分身的身份，不是附属数据
2. **文件是活的** — lifecycle 系统可以直接修改 workspace 里的文件，下次对话自动生效
3. **manifest 是元数据** — AgentManifest 描述运行参数（模型、资源限制、能力），不包含分身身份内容
4. **动态组装** — system prompt 每次对话时从文件构建，不预存

---

## 0.4 分身结构

分身 = 人格 + 指令 + 知识 + 技能 + 子代理。五个部分同级，共同定义分身是什么。

```
~/.opencarrier/workspaces/<name>/
├── agent.toml           # opencarrier agent manifest（模型、工具、资源限制）
├── profile.md           # 分身档案（frontmatter: name/description/source/tags）
├── SOUL.md              # 人格 — "你是谁"（性格、语气、说话方式、边界）
├── system_prompt.md     # 行为指令 — "你怎么做事"（规则、能力、工作流程）
├── MEMORY.md            # 知识索引（始终加载到上下文）
├── data/knowledge/      # 参考资料（LLM 按需加载）
├── skills/              # 能力模块（when_to_use + allowed_tools + 工作流 prompt）
│   ├── spreadsheet.md   # 扁平格式
│   └── vehicle-match/   # 目录格式（带 scripts/）
│       ├── SKILL.md
│       └── scripts/
│           └── search.toml
├── agents/              # 子代理（可派出去干活的专门角色）
│   ├── code-reviewer.md # 扁平格式
│   └── code-architect/  # 目录格式（带 scripts/）
│       ├── AGENT.md
│       └── scripts/
│           └── analyze.toml
├── memory/              # 运行时记忆
├── sessions/            # 会话历史
├── output/              # 分身生成的工作产物
├── logs/                # 运行日志
└── history/             # 知识版本历史 (versions.jsonl)
```

### 五个核心文件的职责边界（v3.0 更新：动态组装 + 子代理）

| 文件 | 定义 | 包含 | 不包含 |
|------|------|------|--------|
| **SOUL.md** | 你是谁 | 性格、语气、说话方式、情绪模式、边界 | 工作规则、流程、参考资料 |
| **system_prompt.md** | 你怎么做事 | 能力、规则、工作方式、输出格式 | 人格描述、FAQ 条目、纯参考文档 |
| **knowledge/** | 你知道什么 | 领域知识、FAQ、产品信息、流程指南 | 行为规则、人格描述 |
| **skills/** | 你会做什么 | when_to_use + allowed_tools + 执行步骤 | 知识事实（放 knowledge/） |
| **agents/** | 你派谁做 | 子代理：独立指令 + 工具白名单 + 模型选择 | 主代理行为规则、知识事实 |

### System Prompt 构建顺序（运行时动态组装）

**关键变更**：system prompt 不再在 .agx 安装时预拼接，而是在每次 agent loop 启动时从 workspace 文件动态构建。这样 lifecycle 系统修改 workspace 文件后，下次对话自动生效。

```
SOUL.md（人格 — 最高优先级）
  → 引导语："体现以上人格和语气"
  → system_prompt.md（行为指令）
  → Skill 目录（所有 skill 的 name + when_to_use，始终注入，很短）
  → Agent 目录（所有 agent 的 name + description + tools，始终注入，很短）
  → Skill 完整 prompt（被激活的 skill 的 body + allowed_tools，按需注入）
  → Agent 完整 prompt（被派出的子代理的 AGENT.md，按需注入）
  → MEMORY.md（知识索引）
  → 相关知识（LLM 按需选择的 knowledge/ 文件）
```

### Skill 详解

Skill 不是"一个功能按钮"——它是分身的行为智能：

```yaml
---
name: spreadsheet
when_to_use: 用户需要创建表格、记录数据、查询表格内容
allowed_tools: [create_spreadsheet, add_rows, query_spreadsheet]
version: 1
usage_count: 15
---

# 表格操作

当用户需要记录或查询数据时，按以下流程操作：
1. 确认用户需要记录什么数据
2. 提取结构化字段
3. 调用 create_spreadsheet 创建表格
4. 调用 add_rows 添加数据
5. 回复用户确认信息
```

一个 Skill = 什么时候激活 + 能用什么工具 + 怎么做。

两个分身用同样的工具（file_write、web_fetch），但因为 Skill 不同，做的事完全不同。**工具是哑的，Skill 是聪明的。**

### Skill 的进化

Skill 和 knowledge 一样有完整的生命周期：

```
诞生: evolution 从对话模式中提取（同类请求 3+ 次 → 自动生成 skill）
成长: compile 阶段优化 prompt、更新 allowed_tools
合并: 两个重叠 skill 合并为一个
淘汰: 30天未激活 → expired → 60天后删除
```

---

## 1. 分身生命周期

```
创建 → 安装 → 运行 → 学习 → 进化 → 评估 → 优化 → 反馈回流
  ↑                                                     │
  └─────────────── Hub 生态闭环 ←───────────────────────┘
```

每个阶段的能力分为三层：

| 层级 | 说明 | 例子 |
|------|------|------|
| **系统能力** | 内核自动运行，分身无感 | 对话后自动进化、知识过期清理、版本管理 |
| **系统工具** | 内核提供，分身可通过 tool_call 调用 | knowledge_import、knowledge_lint、clone_evaluate |
| **分身 Skill** | 分身自带，定义在 skills/ 目录，是分身身份的一部分 | handle-refund、generate-quote、vehicle-match |

---

## 2. 系统能力（内核自动运行）

### 2.1 对话后自动进化（Evolution）

**来源**: openclone `evolution.rs`

每次分身对话结束后，内核在后台自动触发知识提取：

1. **本地预过滤**（零成本，不需要 LLM）：
   - 跳过短回复（< 20 字）
   - 跳过无意义输入（"ok"、"谢谢"、"好的"等硬编码列表）
   - 跳过超短输入（< 4 字符）
2. **LLM 分析**（仅对非平凡对话）：
   - 提取新知识条目 → 写入 `knowledge/`
   - 发现知识缺口 → 追加到 `MEMORY.md`
   - 去重（按文件名）
3. **反馈生成**（forked 分身）：
   - LLM 匿名化处理（替换姓名、电话、公司、价格、地址）
   - 保存到 `feedback/`

**配置**:
```toml
[clone_lifecycle]
evolution_enabled = true
evolution_cooldown_turns = 5    # 每 N 轮对话触发一次
evolution_max_per_hour = 6     # 每小时最多触发次数
```

### 2.2 知识生命周期管理

**来源**: openclone `compile.rs` 的膨胀控制

自动维护知识库健康，防止无限膨胀：

1. **两阶段过期**：
   - `stale_days = 30` — 30 天未修改 → 标记 `status: "expired"`
   - `delete_days = 60` — 过期 60 天 → 自动删除
2. **膨胀控制**：
   - Jaccard 相似度检测重复知识 → LLM 确认 → 合并
   - 超容量时 → LLM 压缩到一半大小
3. **自动编译**：
   - 扫描缺少 `description`/`tags` 的文件 → LLM 生成 → 写回 frontmatter
   - 重建 `MEMORY.md` 索引

**配置**:
```toml
[clone_lifecycle]
bloat_stale_days = 30
bloat_delete_days = 60
auto_compile = true
compile_interval_hours = 24
```

### 2.3 知识版本管理

**来源**: openclone `version.rs`

每次知识变更自动记录，支持回滚和审计：

- JSONL 格式版本日志（`history/versions.jsonl`）
- 每条记录：action（create/update/delete/verify/rollback）、before/after 内容、来源、验证状态
- `evolution` 来源的知识需要人工验证，`user` 来源的自动验证
- 支持回滚到任意版本

---

## 3. 系统工具（分身可调用）

这些工具由内核提供，所有分身都可以通过 tool_call 使用。

### 3.1 数据摄入

**来源**: openclone `import.rs` + `chat_parse.rs`

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `knowledge_import` | 导入文件/URL/目录到分身知识库 | 部分（URL 提取、Agent 分析） |
| `chat_parse` | 解析聊天记录（自动识别平台格式） | 否 |

支持的数据类型：
- **聊天记录** — WeChat/WhatsApp/DingTalk/Telegram JSON，自动检测平台（按字段名路由），20 条消息一组
- **FAQ** — tab/逗号分隔的 Q&A 对
- **文档** — 按段落切分
- **URL** — 抓取 → 提取正文 → Markdown
- **Obsidian Web Clips** — 按 ## 标题切分
- **Agent Markdown** — LLM 分析提取 SOUL/personality/system_prompt/knowledge
- **指定发送者风格** — 模糊名字匹配 + 前/中/后采样

### 3.2 知识管理

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `knowledge_add` | 添加知识条目 | 否 |
| `knowledge_remove` | 删除知识条目（模糊匹配） | 否 |
| `knowledge_search` | 语义/关键词搜索知识 | 否 |
| `knowledge_list` | 列出所有知识文件 | 否 |
| `knowledge_compile` | 为知识文件生成 description/tags | 是 |

### 3.3 健康维护

**来源**: openclone `health.rs`

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `knowledge_lint` | 规则检查知识库健康 | 否 |
| `knowledge_heal` | 自动修复知识库问题 | 部分（补 description） |

lint 检查项（全部确定性）：
- 空文件/空内容
- 缺少 frontmatter
- 缺少 description
- 重复标题（标准化后比较）
- MEMORY.md 与实际文件不同步
- `[待补充]` 占位符

heal 修复项：
- 重建 MEMORY.md 索引（确定性）
- 删除空文件（确定性）
- 补充 frontmatter 模板（确定性）
- 为缺 description 的文件生成描述（LLM）

### 3.4 质量评估

**来源**: openclone `evaluate.rs`

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `clone_evaluate` | 评估分身质量 | 是 |

评估维度：
- **确定性指标**（脚本计算）：知识文件数、总字数、技能覆盖率、system_prompt 长度、MEMORY.md 同步率
- **定性评估**（LLM）：模拟一轮对话，评估回复质量、知识准确性、人格一致性

### 3.5 打包导出

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `clone_export` | 导出分身为 .agx 文件 | 否 |
| `clone_install` | 从 .agx 安装分身 | 否 |
| `clone_list` | 列出已安装分身 | 否 |

### 3.6 反馈回流

**来源**: openclone `feedback.rs`

| 工具名 | 功能 | 需要 LLM |
|--------|------|----------|
| `feedback_collect` | 收集分身的经验反馈 | 否 |
| `feedback_anonymize` | 匿名化反馈内容 | 是 |
| `feedback_push` | 推送反馈到 Hub | 否 |

---

## 4. 实现分层

```
┌─────────────────────────────────────────────────────────┐
│                     分身 (Agent)                         │
│  通过 tool_call 调用系统工具                              │
│  系统能力自动运行，分身无感                                │
└───────────────────────┬─────────────────────────────────┘
                        │ tool_call
                        ▼
┌─────────────────────────────────────────────────────────┐
│                 系统工具层 (Tools)                        │
│  knowledge_import / knowledge_compile / knowledge_lint   │
│  knowledge_heal / clone_evaluate / clone_export          │
│  feedback_collect / feedback_anonymize / feedback_push    │
└───────────────────────┬─────────────────────────────────┘
                        │ 调用
                        ▼
┌─────────────────────────────────────────────────────────┐
│                 脚本层 (Scripts)                          │
│  纯计算/数据转换，不需要 LLM                               │
│  chat_parser / faq_parser / knowledge_indexer            │
│  knowledge_linter / bloat_controller / agx_packer        │
│  version_logger / anonymizer                             │
└───────────────────────┬─────────────────────────────────┘
                        │
                        ▼
┌─────────────────────────────────────────────────────────┐
│              LLM 调用层 (由内核管理)                       │
│  编译（生成 description/tags）                            │
│  进化（提取知识 + 发现缺口）                               │
│  评估（定性评估）                                         │
│  匿名化（替换敏感信息）                                    │
└─────────────────────────────────────────────────────────┘
```

### 什么放脚本，什么放工具

| 放脚本 | 放工具 |
|--------|--------|
| 纯计算、数据转换 | 需要访问分身 workspace 的操作 |
| 不需要知道分身上下文 | 需要知道当前分身是谁 |
| 可独立测试 | 需要内核协调（如 LLM 调用） |
| 例子：聊天记录解析器、Jaccard 计算、JSONL 写入 | 例子：knowledge_import、clone_evaluate |

### 什么放系统能力（自动），什么放系统工具（按需）

| 放系统能力（自动） | 放系统工具（按需） |
|-------------------|-------------------|
| 对话后必须做的 | 用户/分身主动要求的 |
| 不影响对话性能（后台执行） | 需要等待结果的 |
| 例子：进化提取、过期清理、版本记录 | 例子：导入数据、健康检查、评估 |

---

## 5. 在代码中的位置

### 需要修改的

#### opencarrier-clone — converter.rs

**现状**：`convert_to_manifest()` 把 SOUL + system_prompt + MEMORY + skill prompt 拼成一个 system_prompt 字符串。

**修改**：
- `convert_to_manifest()` 不再拼接 system_prompt，只返回 CloneData 的元信息（name、model、resources、capabilities）
- AgentManifest.model.system_prompt 设为空字符串或简短指令
- `install_clone_to_workspace()` 继续负责把文件写入 workspace

#### opencarrier-kernel — kernel.rs + agent loop

**现状**：spawn_agent 用 manifest 里的死 system_prompt 字符串，ensure_workspace() 不创建 knowledge/ 目录。

**修改**：
- `ensure_workspace()` 增加 `data/knowledge/` 目录
- 新增 `build_system_prompt(workspace)` 函数，从 workspace 文件动态组装：
  1. 读 SOUL.md → 作为人格层
  2. 读 system_prompt.md → 作为行为指令层
  3. 扫描 skills/ → 注入 skill 目录（name + when_to_use）
  4. 读 MEMORY.md → 作为知识索引
  5. 按需加载 data/knowledge/ 中的文件
- agent loop 调用 `build_system_prompt()` 而非用 manifest.system_prompt
- Skill 激活：当 LLM 决定使用某个 skill 时，注入完整 skill prompt + allowed_tools

### 新增 crate: `opencarrier-lifecycle`

```
crates/opencarrier-lifecycle/
├── src/
│   ├── lib.rs              # 模块导出
│   ├── evolution.rs         # 对话后自动进化
│   ├── compile.rs           # 知识编译（description/tags 生成 + 膨胀控制）
│   ├── bloat.rs             # 膨胀控制（Jaccard + 过期策略）
│   ├── health.rs            # 知识健康检查（lint + heal）
│   ├── version.rs           # 知识版本管理（JSONL 日志）
│   ├── import.rs            # 数据摄入（调用解析脚本）
│   ├── parsers/
│   │   ├── mod.rs
│   │   ├── chat.rs          # 聊天记录解析（多平台）
│   │   ├── faq.rs           # FAQ 解析
│   │   ├── document.rs      # 文档解析
│   │   └── url.rs           # URL 抓取
│   ├── evaluate.rs          # 分身质量评估
│   ├── feedback.rs          # 反馈收集 + 匿名化
│   └── tools.rs             # 系统工具注册（ToolDefinition 导出）
└── Cargo.toml
```

### 依赖关系

```
opencarrier-lifecycle
  ├── opencarrier-types    # 共享类型
  ├── opencarrier-memory   # 存储访问
  └── opencarrier-runtime  # LLM 调用（KernelHandle trait）
```

### 集成点

1. **opencarrier-kernel** — 启动时注册生命周期系统，对话结束后调用 `evolution::post_conversation()`
2. **opencarrier-runtime** — 系统工具注册到 tool_runner，分身可通过 tool_call 调用
3. **opencarrier-clone** — 安装分身时初始化 `history/` 目录

---

## 6. 与现有系统的关系

### 不变的

- **opencarrier-memory** — 继续负责 SQLite 存储、语义搜索、会话管理
- **opencarrier-runtime** — 继续负责 agent loop、LLM 驱动、工具执行
- **opencarrier-clone** — 继续负责 .agx 加载/安装

### 新增的

- **opencarrier-lifecycle** — 全新的 crate，实现本文档描述的所有能力

### 需要修改的

- **opencarrier-clone** — `converter.rs` 不再拼接 system_prompt，只写文件到 workspace
- **opencarrier-kernel** — 集成 lifecycle 系统 + 动态 system prompt 构建 + ensure_workspace 增加 knowledge/
- **opencarrier-runtime** — 注册 lifecycle 系统工具到 tool_runner + skill 两步激活机制

---

## 7. 开发优先级

### P0 — 核心（让分身能学习） ✅ 已完成

1. `evolution.rs` — 对话后自动进化（这是分身"活"的关键）
2. `parsers/chat.rs` — 聊天记录解析（数据摄入的基础）
3. `version.rs` — 知识版本管理（安全网，进化前必须先有版本记录）

### P1 — 维护（让分身保持健康） ✅ 已完成

4. `health.rs` — 知识 lint + heal
5. `bloat.rs` + `compile.rs` — 膨胀控制 + 自动编译
6. `parsers/faq.rs` + `parsers/document.rs` — 更多数据类型

### P2 — 生态（让分身反哺 Hub） ✅ 已完成

7. `evaluate.rs` — 质量评估
8. `feedback.rs` — 反馈收集 + 匿名化 + 推送

### P3 — 知识品质增强（借鉴 Graphify） — Phase 12 ✅ 已完成

借鉴 [Graphify](https://github.com/gradient-ai/graphify) 的知识图谱理念，增强分身知识的结构化程度和可信度。

---

## 8. Phase 12：知识品质增强（借鉴 Graphify）

> 版本: 3.0 新增 | 日期: 2026-04-12

### 8.1 动机

Phase 10-11 实现了完整的 lifecycle 系统和系统工具注册，但知识的**可信度追踪**和**增量处理**还不足：

1. **分不清知识的来源可信度** — evolution 产出的知识和用户手写的知识在 MEMORY.md 中没有区分
2. **compile 全量扫描** — 每次编译扫描所有文件，知识多了很慢
3. **没有结构性验证** — knowledge 文件的 frontmatter 缺字段不影响运行，但影响质量
4. **没有响应式维护** — workspace 变化后需要手动触发 compile/health

### 8.2 知识置信度标签

**借鉴**: Graphify 的 EXTRACTED / INFERRED / AMBIGUOUS 三级置信度

每个知识文件的 frontmatter 增加 `confidence` 字段：

```yaml
---
name: refund-policy
description: 退货退款政策
confidence: INFERRED    # EXTRACTED | INFERRED | AMBIGUOUS
---

# 退货政策
...
```

**置信度规则**：

| 来源 | 默认置信度 | 升级条件 |
|------|-----------|---------|
| evolution 自动提取 | `INFERRED` | 用户 verify → `EXTRACTED` |
| `knowledge_add` tool | `EXTRACTED` | — |
| `knowledge_import` tool (Tier 1) | `INFERRED` | 用户 verify → `EXTRACTED` |
| 用户直接编辑文件 | `EXTRACTED` | — |
| compile 压缩/合并 | 继承最低的 | 用户 verify → `EXTRACTED` |
| LLM 评估不确定 | `AMBIGUOUS` | 用户 verify → `EXTRACTED` |

**MEMORY.md 展示**：

```markdown
## 知识

- [INFERRED] refund-policy — 退货退款政策 (refund-policy.md)
- [EXTRACTED] faq — 常见问题 (faq.md)
- [AMBIGUOUS] pricing-note — 定价说明 (pricing-note.md)
```

**影响范围**：
- `evolution.rs` — `write_knowledge()` 写入时加 `confidence: INFERRED`
- `version.rs` — `verify_version()` 同时升级 confidence 为 EXTRACTED
- `health.rs` — lint 增加 AMBIGUOUS 数量警告；heal 增加 verify 提示
- `evaluate.rs` — 评分增加 EXTRACTED/INFERRED 比例
- `compile.rs` — 重建 MEMORY.md 时带 confidence 标签
- `tool_runner.rs` — `knowledge_add` 写入时加 `confidence: EXTRACTED`
- `tool_runner.rs` — `knowledge_import` Tier 1 结果加 `confidence: INFERRED`

### 8.3 增量编译（Manifest）

**借鉴**: Graphify 的 `detect_incremental()` — JSON manifest 记录文件 hash + mtime，只处理变化文件

在 workspace 的 `.lifecycle/` 目录存储编译状态：

```
workspace/
├── .lifecycle/
│   └── manifest.json      # { "refund-policy.md": {"hash": "sha256...", "compiled_at": "..."} }
```

**增量 compile 流程**：

1. 加载 `.lifecycle/manifest.json`
2. 扫描 `data/knowledge/`，计算每个文件的 SHA256
3. 对比 manifest：只处理新增 / hash 变化的文件
4. 处理完成后更新 manifest

**与现有 content-hash dedup 的关系**：
- 现有的 content-hash dedup 是编译内部的单文件前后对比（跳过无变化的重写）
- 新的 manifest 是编译外部的多文件过滤（跳过整个文件的编译）
- 两者配合：manifest 先过滤出变化的文件，然后 content-hash 处理内部去重

**改动文件**：
- `compile.rs` — 新增 `load_manifest()` / `save_manifest()` / `find_changed_files()`
- `compile.rs` — `run_compile()` 开头调用增量过滤

### 8.4 知识 Schema 验证

**借鉴**: Graphify 的 `validate.py` — 结构化验证提取结果

在 `health.rs` 的 lint 检查中增加 schema 验证：

**必填字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | string | 知识标题，非空 |
| `description` | string | 简短描述，非空 |

**可选字段**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `confidence` | string | 合法值：EXTRACTED / INFERRED / AMBIGUOUS |
| `tags` | string[] | 标签列表 |
| `status` | string | 合法值：active / expired |

**验证规则**：
- frontmatter 必须存在
- `name` 不能为空，不能包含 `[待补充]`
- `confidence` 如果存在，必须是三个合法值之一
- 双层格式必须有 `---` 分隔符（compiled truth + timeline）
- compiled truth 段不能为空

**改动文件**：
- `health.rs` — 新增 `validate_schema()` 检查，集成到 `check_health()` 的 IssueKind 枚举
- 新增 `IssueKind::InvalidSchema` 变体

### 8.5 Workspace Watch 模式

**借鉴**: Graphify 的 `watch.py` — 文件变化触发自动重建

监听 `data/knowledge/` 目录变化，debounce 后自动触发 health check。

**实现方式**：
- kernel.rs 启动时 spawn 一个 watch 任务（类似现有的 config hot-reload）
- 使用 `notify` crate 监听文件变化
- debounce 3 秒（避免短时间多次变化触发多次）
- 变化检测后运行 `check_health()` + `update_memory_index()`
- 如果配置了 `auto_compile`，则运行增量 compile

**配置**：

```toml
[clone_lifecycle]
watch_enabled = true
watch_debounce_secs = 3
```

**改动文件**：
- `opencarrier-kernel/src/kernel.rs` — 新增 `start_knowledge_watcher()` 方法
- `opencarrier-kernel/Cargo.toml` — 添加 `notify` 依赖

---

### 8.6 实现优先级

| 优先级 | 功能 | 改动范围 | 复杂度 |
|--------|------|---------|--------|
| P3.1 | 知识置信度标签 | evolution + version + health + evaluate + compile + tool_runner | 中 |
| P3.2 | 增量编译 Manifest | compile.rs | 低 |
| P3.3 | 知识 Schema 验证 | health.rs | 低 |
| P3.4 | Workspace Watch | kernel.rs | 中 |

### 8.7 与现有系统的关系

**不改的**：
- `opencarrier-lifecycle` 的模块结构不变（evolution/compile/health/version/evaluate/feedback/parsers）
- `opencarrier-runtime` 的 tool_runner 注册不变（6 个已注册工具）
- API 端点不变

**改的**：
- `opencarrier-lifecycle` 各模块增加 confidence 字段处理
- `opencarrier-kernel` 增加 watch 任务

---

## 9. 参考

- openclone-core `evolution.rs` — 进化逻辑（pre-filter + LLM 分析）
- openclone-core `compile.rs` — 编译 + 膨胀控制
- openclone-core `import.rs` + `chat_parse.rs` — 数据摄入
- openclone-core `health.rs` — lint + heal
- openclone-core `version.rs` — JSONL 版本管理
- openclone-core `evaluate.rs` — 质量评估
- openclone-core `feedback.rs` — 匿名化 + 推送
- [Graphify](https://github.com/gradient-ai/graphify) — 知识图谱工具（置信度标签、增量处理、文件监听、Schema 验证的灵感来源）
- [分身工厂产品愿景](../../openclone/docs/CLONE-FACTORY.md)
- [Skill 系统设计](../../openclone/docs/SKILL-SYSTEM.md)
