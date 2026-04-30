# OpenCarrier：分身操作系统 —— 不只是 Agent 框架

> 当其他框架还在思考"怎么让 LLM 调用工具"时，OpenCarrier 已经在构建"数字生命的运行环境"。

## 什么是 OpenCarrier

OpenCarrier 是一个**开源的分身操作系统**（Agent Operating System），用 Rust 从零构建。

它不是聊天机器人的封装，不是 LLM 的 Python 胶水代码，而是一个完整的操作系统——只不过它运行的不是程序，而是**分身（Clone）**。

每个分身是一个独立的数字实体：有自己的人格、知识、技能和工作空间。你从 Hub 下载分身，它在你的机器上自主运行、学习、进化。

```
用户自部署 OpenCarrier → 从 Hub 下载分身 → 分身自主运行、学习、进化
```

## 五层架构：一个完整的数字生命系统

OpenCarrier 由五个核心层组成，每层职责清晰，共同构成一个分身所需的全部能力：

```
┌──────────────────────────────────────────┐
│  分身 (Clone) — WHO: 身份 + 工作空间     │
├──────────────────────────────────────────┤
│  大脑 (Brain) — THINK: LLM 智能路由      │
├──────────────────────────────────────────┤
│  工具 (Tool) — DO: 内置系统能力          │
├──────────────────────────────────────────┤
│  MCP — EXTEND: 外部工具接入              │
├──────────────────────────────────────────┤
│  记忆 (Memory) — REMEMBER: 生命周期管理   │
└──────────────────────────────────────────┘
```

### 分身层 — 你是谁

分身不是配置文件，而是一个完整的数字身份：

```
~/.opencarrier/workspaces/<name>/
├── SOUL.md              ← 人格：性格、语气、边界
├── system_prompt.md     ← 行为指令：怎么做事
├── MEMORY.md            ← 知识索引（始终加载）
├── data/knowledge/      ← 知识库（按需加载）
├── skills/              ← 技能（按需激活）
├── agents/              ← 子代理（可派出去干活）
└── agent.toml           ← 运行参数：模型、资源、能力
```

**同样的工具 + 同样的大脑，不同的分身做完全不同的事。** 一个客服分身和一个程序员分身，用的是同一个 OpenCarrier 实例，但行为、知识、输出风格完全不同。

关键设计：Workspace 即分身。修改 workspace 里的文件，下次对话自动生效，不需要重新安装。

### 大脑层 — 怎么思考

大脑负责 LLM 调用的智能路由：

- **三层路由**：Provider → Endpoint → Modality
- **20+ Provider**：Anthropic、OpenAI、Gemini、Groq、DeepSeek、OpenRouter、Ollama、vLLM 等
- **熔断器**：连续失败 3 次自动熔断，60 秒冷却
- **热重载**：修改配置即时生效，无需重启

大脑不关心"谁在调用"，只关心"用哪个模型"。所有分身共享同一个大脑。

### 工具层 — 能做什么

系统级内置工具，所有分身共享：

| 类别 | 工具 |
|------|------|
| 文件 | file_read, file_write, file_list |
| 网络 | web_fetch, web_search |
| 执行 | shell_exec |
| 浏览器 | browser_* 系列 |
| 知识 | knowledge_add, knowledge_import, knowledge_compile |
| 记忆 | memory_store, memory_recall, user_profile |

通过 `capabilities.tools` 白名单控制每个分身能用哪些工具。

### MCP 层 — 还能扩展什么

外部工具接入层（Model Context Protocol）：

- **连接方式**：stdio（本地进程）或 SSE（HTTP 长连接）
- **命名空间**：`mcp_{server}_{tool}` 防冲突
- **per-agent 过滤**：每个分身可选择使用哪些 MCP 服务器
- **健康监控**：后台自动 ping，断开自动重连
- **热重载**：修改配置即时生效

### 记忆层 — 记得什么

独立的记忆生命周期管理层，跨分身的基础设施：

**自动整理（系统级）**：
- **ConsolidationEngine** — 每 24h 对 7 天未访问的记忆降低置信度
- **Session Compaction** — 三阶段 LLM 压缩（>30 条消息触发）
- **Context Overflow Recovery** — 4 级渐进恢复
- **Knowledge Bloat Control** — 两步过期（30 天 stale → 60 天删除）

**记忆存储（per-agent）**：
- **Structured KV** — JSON 键值存储，写入不可变
- **Semantic** — 向量嵌入 + 余弦相似度搜索
- **Knowledge Graph** — 实体-关系图谱
- **Canonical Session** — 跨渠道持久会话

## 动态 System Prompt 构建

System prompt 不在安装时预拼接，而是每次对话从 workspace 文件动态构建：

```
SOUL.md（人格 — 最高优先级）
  → "体现以上人格和语气"
  → system_prompt.md（行为指令）
  → Skill 目录（所有 skill 的 name + when_to_use）
  → Agent 目录（子代理定义）
  → 激活的 Skill 完整 prompt
  → MEMORY.md（知识索引）
  → 相关知识（LLM 按需选择）
```

这样 lifecycle 系统修改 workspace 文件后，不需要重新安装分身，下次对话自动生效。

## 系统 = 身体，分身 = 人格

OpenCarrier 把"平台能力"和"分身智能"彻底分离：

**系统（身体）** — 分身无感，自动运行：
- 对话后自动进化（提取新知识、发现缺口）
- 知识生命周期（过期清理、膨胀控制、重复合并）
- 知识版本管理（变更自动记录，支持回滚）
- 反馈回流（匿名化经验推送回 Hub）

**分身（人格）** — 由文件定义：
- SOUL.md：性格、语气、边界
- system_prompt.md：能力、规则、工作方式
- skills/：什么时候做什么、怎么做
- knowledge/：知道什么

这个分离是 OpenCarrier 与其他框架最本质的区别。

## 16 层安全防御

| 层 | 系统 | 功能 |
|----|------|------|
| 1 | WASM Dual-Metered Sandbox | 燃料计量 + epoch 中断，watchdog 杀死失控代码 |
| 2 | Merkle Hash-Chain Audit | 每个操作密码学链接到前一个，篡改即断裂 |
| 3 | Taint Tracking | 信息流标记传播，追踪 secrets 从源头到出口 |
| 4 | Ed25519 Manifest Signing | 分身身份和能力集密码学签名 |
| 5 | SSRF Protection | 阻断私有 IP、云元数据端点、DNS rebinding |
| 6 | Secret Zeroization | API key 用 `Zeroizing<String>` 自动擦除 |
| 7 | Capability Gates | RBAC 能力门控，分身声明工具，内核强制执行 |
| 8 | Path Traversal Prevention | 规范化 + symlink 逃逸防护 |
| 9 | Loop Guard | SHA256 工具循环检测 + 熔断器 |
| 10 | Session Repair | 7 阶段消息历史修复 |
| + | GCRA Rate Limiter, Security Headers, Prompt Injection Scanner, Subprocess Sandbox, Health Redaction, OFP HMAC Auth |

## 与其他框架的区别

| | OpenCarrier | OpenClaw | Hermes Agent |
|---|---|---|---|
| **定位** | 分身操作系统 | 本地 Agent 框架 | 自学习 Agent 框架 |
| **语言** | Rust (14 crates) | Node.js | Python |
| **核心概念** | 分身 = 数字实体 | Agent + Gateway | AIAgent Loop |
| **自进化** | 系统级自动运行 | 无 | GEPA 自学习 |
| **P2P 互联** | OFP 原生协议 | 无 | 无 |
| **安全** | 16 层系统级 | 权限级联 | 权限分离 |
| **记忆** | SQLite + 语义 + 图谱 | Markdown 文件 | SQLite + FTS5 |

**一句话**：OpenClaw 是"让 LLM 能调工具的本地聊天框架"，Hermes 是"会自己写技能的自学习代理"，OpenCarrier 是"运行数字实体的操作系统"。

## 为什么是 Rust

- **性能**：单二进制，启动快，内存占用低
- **安全**：所有权系统杜绝数据竞争和空指针
- **并发**：Tokio 异步运行时，轻松处理数千并发连接
- **部署**：一个二进制文件，无依赖，跨平台

## 快速开始

```bash
# 1. 构建
cargo build --release -p opencarrier-cli

# 2. 初始化
./target/release/opencarrier init

# 3. 启动守护进程
./target/release/opencarrier start

# 4. Dashboard
open http://localhost:4200
```

## 总结

OpenCarrier 不是在现有的 LLM 调用链上加一层包装，而是从头构建了一个运行环境。

在这个环境里：
- 分身有自己的身份、知识和进化能力
- 系统负责维持生命（清理、修复、版本记录）
- 安全是系统设计的一部分，不是后期补丁
- 互联是原生能力，不是附加功能

**这不是一个 Agent 框架，这是一个 Agent 的操作系统。**
