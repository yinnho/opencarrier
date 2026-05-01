<h1 align="center">OpenCarrier</h1>
<h3 align="center">分身操作系统 — Agent Operating System</h3>

<p align="center">
  Open-source Agent OS built in Rust. 10 crates. 1493 tests.<br/>
  <strong>分身 + 大脑 + 工具 + MCP + 记忆 — 五层架构，一个二进制</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/tests-1493-brightgreen?style=flat-square" alt="Tests" />
</p>

---

## What is OpenCarrier?

OpenCarrier is an **open-source Agent Operating System** — 不是聊天框架，不是 LLM 的 Python 包装，而是一个从零开始用 Rust 构建的完整 Agent 操作系统。

核心理念：**分身（Clone）**。每个分身是一个独立的数字实体，拥有自己的人格、知识、技能和工作空间。分身从 Hub 下载，在本地运行，能学习、进化、自我维护。

```bash
opencarrier init      # 初始化配置
opencarrier start     # 启动守护进程
# Dashboard: http://localhost:4200
```

---

## 五层架构

OpenCarrier 由五个核心层组成，每层职责清晰：

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

### 分身层 (Clone) — WHO

分身是系统的核心实体，决定"做什么"：

```
~/.opencarrier/workspaces/<name>/
├── SOUL.md              # 人格 — "你是谁"
├── system_prompt.md     # 行为指令 — "你怎么做事"
├── MEMORY.md            # 知识索引（始终加载）
├── data/knowledge/      # 知识库（按需加载）
├── skills/              # 技能（per-agent 自定义）
├── agents/              # 子代理（可派出去干活）
└── agent.toml           # 运行参数（模型、资源、能力）
```

同样的工具 + 同样的大脑，不同的分身做完全不同的事。

关键设计：
- **Workspace 即分身** — workspace 里的文件就是分身的身份
- **动态组装** — system prompt 每次对话从文件构建，不预存
- **Lifecycle 系统** — 对话后自动进化、知识过期清理、版本管理

### 大脑层 (Brain) — THINK

大脑负责 LLM 调用的智能路由，配置在 `brain.json` 中：

```
Provider (OpenAI / Anthropic / Gemini / ...)
  └── Endpoint (gpt-4o / claude-sonnet / ...)
        └── Modality (chat / vision / tools / ...)
```

- **三层路由**: Provider → Endpoint → Modality
- **熔断器**: 连续失败 >= 3 次触发熔断，60 秒冷却
- **热重载**: 修改 `brain.json` 即时生效，无需重启
- **20+ Provider**: Anthropic, OpenAI, Gemini, Groq, DeepSeek, OpenRouter, Ollama, vLLM 等

### 工具层 (Tool) — DO

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

### MCP 层 — EXTEND

外部工具接入层（Model Context Protocol）：

- **连接方式**: stdio（本地进程）或 SSE（HTTP 长连接）
- **命名空间**: `mcp_{server}_{tool}` 防冲突
- **per-agent 过滤**: 每个分身可通过白名单选择使用哪些 MCP 服务器
- **健康监控**: 后台自动 ping，断开自动重连
- **热重载**: 修改配置即时生效

### 记忆层 (Memory) — REMEMBER

独立的记忆生命周期管理层，跨分身的基础设施：

**自动整理（系统级）**：
- **ConsolidationEngine** — 每 24h 对 7 天未访问的记忆降低 confidence
- **Session Compaction** — 三阶段 LLM 压缩（>30 条消息或 >70% context window 触发）
- **Context Overflow Recovery** — 4 级渐进恢复
- **Knowledge Bloat Control** — 两步过期（30 天 stale → 60 天删除）

**记忆存储（per-agent）**：
- **Structured KV** — JSON 键值存储，写入不可变
- **Semantic** — 向量嵌入 + 余弦相似度搜索
- **Knowledge Graph** — 实体-关系图谱
- **Canonical Session** — 跨渠道持久会话

---

## Crate 结构

```
opencarrier-types          共享类型 (Agent, Capability, Config, Message, Tool...)
opencarrier-memory         SQLite 记忆层 (KV / Semantic / Knowledge Graph / Session)
opencarrier-runtime        Agent loop + 3 LLM drivers + 23 tools + MCP + A2A
opencarrier-kernel         内核: 组装所有子系统, RBAC, 调度, 触发器
opencarrier-api            REST/WS/SSE API + Dashboard
opencarrier-cli            CLI (init/start/agent/chat/config/mcp)
opencarrier-lifecycle      分身生命周期: 进化, 编译, 健康, 评估, 版本
opencarrier-clone          分身管理: Hub 下载, .agx 加载, workspace 安装
opencarrier-skills         Bundled skills
opencarrier-plugin-sdk     Plugin SDK (crates.io) for external integrations
```

---

## 插件系统

消息渠道通过独立插件实现，插件通过 Plugin SDK（[crates.io](https://crates.io/crates/opencarrier-plugin-sdk)）开发，动态加载：

| 插件 | 说明 |
|------|------|
| wecom | 企业微信 |
| weixin | 微信公众号 |
| feishu | 飞书 |
| bilibili | B站 |
| xiaohongshu | 小红书 |
| zhihu | 知乎 |
| twitter | Twitter/X |
| reddit | Reddit |

插件仓库: [opencarrier-plugins](https://github.com/yinnho/opencarrier-plugins)

---

## 安全体系

16 层安全防御，每层独立可测试：

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

---

## Quick Start

```bash
# 1. Build
cargo build --release -p opencarrier-cli

# 2. Initialize
./target/release/opencarrier init

# 3. Start daemon
./target/release/opencarrier start

# 4. Dashboard
open http://localhost:4200
```

---

## Development

```bash
cargo build --workspace --lib          # 编译
cargo test --workspace                 # 1493 tests
cargo clippy --workspace --all-targets -- -D warnings  # 0 warnings
```

---

## License

MIT
