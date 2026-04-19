# OpenCarrier 架构原则

> **版本**: v3.0
> **日期**: 2026-04-11
> **状态**: 已确立

---

## 1. 核心架构：opencarrier 是分身操作系统

### 1.1 一句话定义

**opencarrier = 分身 OS**。每个用户自部署一个 opencarrier 实例，从 Hub 下载分身运行。

```
用户自部署 opencarrier → 从 Hub 下载分身 → 分身自主运行、学习、进化
```

### 1.2 分身 = 人格 + 指令 + 知识 + 技能 + 子代理

分身不是静态的 agent，而是能学习、成长的数字实体。五个部分同级，共同定义分身是什么：

| 组成 | 文件 | 定义 | 包含 | 不包含 |
|------|------|------|------|--------|
| 人格 | SOUL.md | 你是谁 | 性格、语气、说话方式、情绪模式、边界 | 工作规则、流程、参考资料 |
| 指令 | system_prompt.md | 你怎么做事 | 能力、规则、工作方式、输出格式 | 人格描述、FAQ 条目、纯参考文档 |
| 知识 | knowledge/*.md + MEMORY.md | 你知道什么 | 领域知识、FAQ、产品信息、流程指南 | 行为规则、人格描述 |
| 技能 | skills/*.md | 你会做什么 | when_to_use + allowed_tools + 执行步骤 | 知识事实（放 knowledge/） |
| 子代理 | agents/*.md | 你派谁做 | 独立执行者：指令 + 工具白名单 + 模型 | 主代理行为规则、知识事实 |

**工具是哑的，Skill 是聪明的。** 两个分身用同样的工具（file_write、web_fetch），但因为 Skill 不同，做的事完全不同。

### 1.3 分身归 Hub 管理

```
Hub (hub.yinnho.cn)  ←→  opencarrier (本地)
     │                        │
     ├─ 发布/搜索/下载         ├─ 运行分身
     ├─ 版本管理               ├─ 自动进化
     ├─ 评分/反馈              ├─ 知识管理
     └─ API Key + 设备绑定     └─ 经验回流
```

- 用户在 Hub 注册账号，创建 API Key
- `opencarrier hub install <name>` 下载分身，绑定设备
- 分身运行中产生的经验可以匿名化后推回 Hub

### 1.4 关键原则

| 原则 | 说明 |
|------|------|
| **一户一备** | 每个用户自部署自己的 opencarrier，不需要多租户隔离 |
| **分身绑定环境** | 企微分身就是企微分身，skill 写死调企微 API，不需要跨平台抽象 |
| **通道 = 管道** | 企微/Telegram/Discord 只是消息通道，分身根据内容决定行为 |
| **平台提供能力** | 学习、进化、维护是平台级能力，不是分身 skill |
| **系统提供机制，分身提供智能** | 系统是身体（自动进化、清理、版本记录），分身是人格（决定行为） |
| **Workspace 即分身** | workspace 里的文件就是分身的身份，不是附属数据 |
| **文件是活的** | lifecycle 系统修改 workspace 文件，下次对话自动生效 |
| **Manifest 是元数据** | AgentManifest 描述运行参数（模型、资源、能力），不含身份内容 |
| **动态组装** | system prompt 每次对话从文件构建，不预存 |

### 1.5 系统与分身的关系

```
系统（opencarrier）                       分身
┌──────────────────────┐               ┌──────────────────────┐
│ 自主功能（分身无感）    │               │ 身份（SOUL.md）        │
│ · 对话后进化           │──自动触发──→  │ 指令（system_prompt）  │
│ · 知识过期清理         │               │ 知识（knowledge/）     │
│ · 版本记录            │               │ 技能（skills/）        │
│                      │               │ 子代理（agents/）      │
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
- **系统工具 = 器官**（可用但分身决定何时用）

### 1.6 Workspace 即分身

分身的 workspace 不只是工作目录——它就是分身本身：

```
~/.opencarrier/workspaces/<name>/
├── SOUL.md              ← 人格（系统 = 身体，这部分 = 性格）
├── system_prompt.md     ← 行为指令（怎么做事情）
├── MEMORY.md            ← 知识索引（始终加载）
├── data/knowledge/      ← 知识库（按需加载）
├── skills/              ← 技能（按需激活）
├── agents/              ← 子代理（可派出去干活的专门角色）
├── agent.toml           ← 运行参数（模型、资源、能力）— 不是身份
├── profile.md           ← 分身档案（名称、描述、来源）
├── history/             ← 知识版本历史
├── sessions/            ← 会话记录
├── memory/              ← 运行时记忆
├── output/              ← 工作产物
└── logs/                ← 运行日志
```

**关键**：lifecycle 系统直接操作这些文件。修改 knowledge/ 中的文件、更新 skills/ 中的技能、整理 MEMORY.md —— 这些操作不需要改 manifest，下次对话自动生效。

### 1.7 动态 System Prompt 构建

System prompt 不在 .agx 安装时预拼接，而是每次 agent loop 启动时从 workspace 文件动态构建：

```
SOUL.md（人格 — 最高优先级）
  → 引导语："体现以上人格和语气"
  → system_prompt.md（行为指令）
  → Skill 目录（所有 skill 的 name + when_to_use，始终注入）
  → Agent 目录（所有 agent 的 name + description + tools，始终注入）
  → Skill 完整 prompt（被激活的 skill 的 body + allowed_tools，按需注入）
  → Agent 完整 prompt（被派出的子代理的 AGENT.md，按需注入）
  → MEMORY.md（知识索引）
  → 相关知识（LLM 按需选择的 knowledge/ 文件）
```

这样 lifecycle 系统修改 workspace 文件后，不需要重新安装分身，下次对话自动生效。

### 1.8 五层系统架构

opencarrier 由五个核心层组成，每层职责清晰：

```
┌──────────────────────────────────────────┐
│  分身 (Clone) — WHO: 身份 + 工作空间     │
│  SOUL.md / agent.toml / skills / knowledge│
├──────────────────────────────────────────┤
│  大脑 (Brain) — THINK: LLM 路由          │
│  Provider → Endpoint → Modality + 熔断   │
├──────────────────────────────────────────┤
│  工具 (Tool) — DO: 内置能力              │
│  file / web / shell / browser / knowledge │
├──────────────────────────────────────────┤
│  MCP — EXTEND: 外部工具接入              │
│  stdio / SSE 连接，per-agent 过滤        │
├──────────────────────────────────────────┤
│  记忆 (Memory) — REMEMBER                │
│  衰减 / 压缩 / 膨胀控制 / 溢出恢复       │
│  semantic + structured + knowledge graph │
└──────────────────────────────────────────┘
```

#### 分身层 (Clone) — WHO

分身是系统的核心实体，决定"做什么"：

- **身份**: SOUL.md（人格）、system_prompt.md（行为指令）
- **技能**: skills/ 目录中的 per-agent 技能定义（唯一能让分身自定义能力的层）
- **知识**: knowledge/ 目录中的知识文件 + MEMORY.md 索引
- **工作空间**: 独立的文件系统沙箱

分身是唯一能让系统"做不同事情"的层。同样的工具 + 同样的大脑，不同的分身做完全不同的事。

#### 大脑层 (Brain) — THINK

大脑负责 LLM 调用的智能路由，配置在 brain.json 中：

- **三层路由**: Provider → Endpoint → Modality
- **熔断器**: 连续失败 ≥ 3 次触发熔断，60 秒冷却后重试
- **热重载**: `Arc<RwLock<Arc<Brain>>>` 三层包装，原子 Arc swap 无缝切换

大脑不关心"谁在调用"，只关心"用哪个模型"。所有分身共享同一个大脑。

#### 工具层 (Tool) — DO

系统级内置工具，所有分身共享，分身改不了：

- **文件**: file_read, file_write, file_list
- **网络**: web_fetch, web_search
- **执行**: shell_exec
- **浏览器**: browser_* 系列
- **知识**: knowledge_add, knowledge_import, knowledge_compile 等
- **记忆**: memory_store, memory_recall, user_profile

通过 `capabilities.tools` 白名单控制哪些分身能用哪些工具。

#### MCP 层 — EXTEND

外部工具接入层：

- **连接方式**: stdio（本地进程）或 SSE（HTTP 长连接）
- **命名空间**: 工具命名为 `mcp_{server}_{tool}` 防冲突
- **per-agent 过滤**: 通过 `mcp_servers` 白名单控制哪些分身能用哪些 MCP 服务器
- **健康监控**: 后台 60 秒 ping 一次，自动重连断开的服务器
- **热重载**: 配置变更时自动重连

Tool 与 MCP 的边界：

| | Tool | MCP |
|---|---|---|
| 来源 | 内置（Rust 代码） | 外部（第三方服务器） |
| 配置 | 不需要 | 需要 config.toml 配置连接 |
| 作用域 | 全局可用 | 全局连接 + per-agent 过滤 |
| 扩展 | 改代码 | 改配置，热重载 |
| 稳定性 | 高（编译时保证） | 低（依赖外部进程） |

#### 记忆层 (Memory) — REMEMBER

独立的记忆生命周期管理层，跨分身的基础设施，不挂在分身下面：

**记忆整理（系统级，自动运行）**：
- **ConsolidationEngine**: 每 24 小时对 7 天未访问的记忆降低 confidence（衰减率 0.1，最低 0.1）
- **Usage 清理**: 每 24 小时删除 90 天前的用量记录

**会话管理（per-agent）**：
- **Session Compaction**: 三阶段 LLM 压缩（完整摘要 → 分块摘要 → 纯截断），>30 条消息或 >70% context window 触发
- **Context Overflow Recovery**: 4 级渐进恢复（中等裁剪 → 激进裁剪 → 工具结果截断 → 报错）
- **Context Guard**: 工具结果动态截断，防止单个结果超过 context window 30%
- **Session Repair**: 消息历史修复（去孤立、去空、去重、重排序）
- **Canonical Session**: 跨渠道持久会话，>100 条自动压缩

**知识膨胀控制（per-clone）**：
- 两步过期：30 天标记 stale → 60 天删除
- 容量裁剪：超限时优先删最冷的文件
- 合并候选：标签重叠度 ≥ 0.7 的文件建议合并

**记忆存储（per-agent）**：
- **Structured KV**: JSON 键值存储，写入不可变（历史版本保留）
- **Semantic**: 向量嵌入 + 余弦相似度搜索
- **Knowledge Graph**: 实体-关系图谱

记忆层是独立的基础设施，有自己的生命周期管理（衰减、压缩、清理），独立于分身的创建和销毁。

---

## 2. 三层能力架构

### 2.1 系统能力（内核自动运行）

平台内置，分身无感，自动触发：

- **对话后自动进化** — 每次对话后台提取新知识、发现知识缺口
- **知识生命周期** — 过期清理、膨胀控制、重复合并
- **知识版本管理** — 变更自动记录，支持回滚和审计
- **反馈回流** — 匿名化经验推送回 Hub

### 2.2 系统工具（内核提供，分身可调用）

平台提供，所有分身通过 tool_call 使用：

| 工具 | 功能 |
|------|------|
| `knowledge_import` | 导入数据（聊天记录/FAQ/文档/URL） |
| `knowledge_compile` | 编译知识（生成 description/tags） |
| `knowledge_lint` | 知识健康检查 |
| `knowledge_heal` | 自动修复知识问题 |
| `clone_evaluate` | 分身质量评估 |
| `clone_export` | 导出为 .agx |
| `clone_list` | 列出已安装分身 |
| `feedback_push` | 推送经验反馈到 Hub |

### 2.3 分身 Skill（分身特有）

Skill 是分身的行为智能，定义在 skills/ 目录中，是分身身份的一部分。格式为 Markdown + YAML frontmatter：

```yaml
---
name: customer-support
when_to_use: 用户咨询退货、换货、售后问题时激活
allowed_tools: [knowledge_search, web_fetch]
version: 1
usage_count: 15
---

# 客户支持流程

当用户咨询售后问题时，按以下流程操作：
1. 确认用户的问题类型（退货/换货/投诉）
2. 搜索知识库中的相关政策
3. 根据政策给出解决方案
4. 记录处理结果
```

一个 Skill = **什么时候激活** + **能用什么工具** + **怎么做**。

Skill 有完整的进化生命周期：从对话模式中诞生（同类请求 3+ 次 → 自动生成），通过 compile 优化，重叠时合并，30 天未激活过期，60 天后删除。

---

## 3. 实现分层原则

```
┌─────────────────────────────────┐
│     Skill（LLM 编排层）          │  需要理解上下文、多步决策
│     clone-import / clone-heal    │  调用 Tool + Script
├─────────────────────────────────┤
│     Tool（原子操作层）            │  单步、确定性
│     knowledge_import / lint     │  内核注册，分身可调用
├─────────────────────────────────┤
│     Script（纯计算层）           │  数据转换，不需要 LLM
│     chat_parser / faq_parser    │  通过 shell_exec 或 Tool 调用
└─────────────────────────────────┘
```

**原则**：
- LLM 只做需要判断的事
- 机械操作都是脚本
- Skill 是编排层

---

## 4. Crate 结构

```
opencarrier-cli            CLI (二进制名: opencarrier)
    |
opencarrier-api            REST/WS/SSE API (Axum 0.8, 76 endpoints)
    |
opencarrier-kernel         内核：组装所有子系统
    |
    +-- opencarrier-runtime     Agent loop, 3 LLM drivers, 23 tools
    +-- opencarrier-channels    40 channel adapters
    +-- opencarrier-clone       .agx 分身管理：Hub 下载、安装、转换
    +-- opencarrier-lifecycle   分身生命周期：进化、编译、健康、评估  ← NEW
    +-- opencarrier-skills      60 bundled skills
    +-- opencarrier-extensions  MCP 扩展系统
    +-- opencarrier-hands       Hands 系统
    +-- opencarrier-wire        OFP P2P 网络
    +-- opencarrier-migrate     迁移工具
    |
opencarrier-memory         SQLite 存储、语义搜索、会话管理
    |
opencarrier-types          共享类型
```

---

## 5. 设计文档索引

| 文档 | 内容 |
|------|------|
| [architecture.md](./architecture.md) | 技术架构：crate 结构、内核启动、agent 生命周期、安全模型 |
| [CLONE-LIFECYCLE-SYSTEM.md](./CLONE-LIFECYCLE-SYSTEM.md) | 分身生命周期系统：进化、编译、健康、评估的详细设计 |
| [skill-development.md](./skill-development.md) | Skill 开发指南 |
| [configuration.md](./configuration.md) | 配置参考 |
| [api-reference.md](./api-reference.md) | API 文档 |
| [channel-adapters.md](./channel-adapters.md) | 40 通道适配器 |

---

**最后更新**: 2026-04-11
**维护者**: 应合网络团队
