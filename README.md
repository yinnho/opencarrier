<h1 align="center">OpenCarrier</h1>
<h3 align="center">扫码即用 — 你的 AI 分身，在微信/飞书/钉钉里等你</h3>

<p align="center">
  开源 Agent 操作系统 | 200+ 个预训练分身 | 微信、企微、飞书、钉钉全平台支持<br/>
  <strong>不装 App、不配环境、不跑命令行。扫个码，机器人就是你的 AI 助理。</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT" />
  <img src="https://img.shields.io/badge/version-0.3.0-blueviolet?style=flat-square" alt="v0.3.0" />
  <img src="https://img.shields.io/badge/clones-200+-brightgreen?style=flat-square" alt="200+ Clones" />
</p>

---

## 怎么用？

打开链接，选一个分身，扫码绑定。**一分钟搞定。**

<p align="center">
  <strong>👉 <a href="https://carrier.yinnho.cn/share">carrier.yinnho.cn/share</a> 👈</strong>
</p>

支持平台：

| 平台 | 入口 | 能力 |
|------|------|------|
| 个人微信 | 扫码绑定 iLink 机器人 | 对话、文章推送 |
| 企业微信 | SmartBot / 应用 / 客服 | 对话、主动推送、群聊 |
| 飞书 | 扫码授权 | 对话、主动推送 |
| 钉钉 | 扫码授权 | 对话、主动推送 |

---

## 200+ 个分身，还在增长

分身（Clone）是 OpenCarrier 的核心——每个分身是独立训练的 AI 角色，有自己的知识、人格和技能。

从 **AI 写作官**、**前端设计师**、**拖延症终结者** 到 **美食探险家**、**情绪日记助手**……200+ 个分身覆盖创作、效率、生活、学习的方方面面。

分身市场：[hub.aginx.net](https://hub.aginx.net)

---

## 社区

<p align="center">
  <img src="docs/wechat-group.png" width="200" alt="微信群" /><br/>
  <sub>扫码加入微信群，聊聊你的 AI 分身</sub>
</p>

---

## Installation

### 一键安装（推荐）

```bash
curl -sSf https://opencarrier.sh | sh
```

国内用户自动 fallback 到 hub.aginx.net 镜像，无需访问 GitHub。

### 手动下载

从 [GitHub Releases](https://github.com/yinnho/opencarrier/releases) 下载对应平台的二进制：

```bash
# Linux x86_64
curl -L https://github.com/yinnho/opencarrier/releases/latest/download/opencarrier-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv opencarrier /usr/local/bin/

# macOS ARM (Apple Silicon)
curl -L https://github.com/yinnho/opencarrier/releases/latest/download/opencarrier-aarch64-apple-darwin.tar.gz | tar xz
sudo mv opencarrier /usr/local/bin/

# macOS x86_64 (Intel)
curl -L https://github.com/yinnho/opencarrier/releases/latest/download/opencarrier-x86_64-apple-darwin.tar.gz | tar xz
sudo mv opencarrier /usr/local/bin/
```

### 从源码编译

```bash
git clone https://github.com/yinnho/opencarrier.git
cd opencarrier
cargo build --release -p opencarrier-cli
cp target/release/opencarrier /usr/local/bin/
```

---

## Quick Start

```bash
# 1. 初始化（自动注册 Hub，生成配置和登录凭据）
opencarrier init

# 2. 启动守护进程（首次启动自动从 Hub 拉取 brain.json）
opencarrier start

# 3. 打开 Dashboard
open http://localhost:4200
```

### 配置 Brain（LLM 路由）

OpenCarrier 使用 `brain.json` 进行 LLM 智能路由，**不是** config.toml。

`opencarrier init` 首次启动时会自动从 Hub 拉取 `brain.json`。如需自定义，编辑 `~/.opencarrier/brain.json`：

```json
{
  "providers": {
    "zhipu": { "api_key_env": "ZHIPU_API_KEY" },
    "deepseek": { "api_key_env": "DEEPSEEK_API_KEY" },
    "ollama": {}
  },
  "endpoints": {
    "zhipu_chat": {
      "provider": "zhipu",
      "model": "glm-4-flash",
      "base_url": "https://open.bigmodel.cn/api/paas/v4",
      "format": "openai"
    },
    "deepseek_chat": {
      "provider": "deepseek",
      "model": "deepseek-chat",
      "base_url": "https://api.deepseek.com/v1",
      "format": "openai"
    },
    "ollama_local": {
      "provider": "ollama",
      "model": "llama3:latest",
      "base_url": "http://localhost:11434/v1"
    }
  },
  "modalities": {
    "chat": { "primary": "zhipu_chat", "fallbacks": ["deepseek_chat"] },
    "fast": { "primary": "ollama_local" }
  }
}
```

三层路由结构：
- **Provider** — 身份 + 凭据（API key 从环境变量读取）
- **Endpoint** — 完整调用单元（provider + model + base_url + format）
- **Modality** — 任务类型 → endpoint 映射（chat/fast/vision...）支持 fallback 链

支持的 format: `openai`（兼容 OpenAI/Groq/DeepSeek/Ollama 等）、`anthropic`、`gemini`

修改 `brain.json` 后**即时生效**，无需重启（热重载）。

> **提示**: API key 通过环境变量设置（如 `export ZHIPU_API_KEY=xxx`），也可写入 `~/.opencarrier/.env`。

### 安装插件

```bash
# 搜索插件
opencarrier plugin search wechat

# 安装插件（从 Hub 下载预编译二进制）
opencarrier plugin install weixin

# 查看已安装插件
opencarrier plugin list

# 重启 daemon 加载插件
opencarrier stop && opencarrier start
```

---

## What is OpenCarrier?

OpenCarrier is an **open-source Agent Operating System** — 不是聊天框架，不是 LLM 的 Python 包装，而是一个从零开始用 Rust 构建的完整 Agent 操作系统。

核心理念：**分身（Clone）**。每个分身是一个独立的数字实体，拥有自己的人格、知识、技能和工作空间。分身从 Hub 下载，在本地运行，能学习、进化、自我维护。

**v0.3.0** 的核心突破：分身不再是 Dashboard 里的抽象概念——你在微信、飞书、钉钉里扫个码，机器人就是你的 AI 助理。聊天就是交互界面。

---

## 五层架构

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
| 知识 | knowledge_add, knowledge_import, knowledge_compile |
| 记忆 | memory_store, memory_recall, user_profile |

### MCP 层 — EXTEND

外部工具接入层（Model Context Protocol）：

- **连接方式**: stdio（本地进程）或 SSE（HTTP 长连接）
- **命名空间**: `mcp_{server}_{tool}` 防冲突
- **per-agent 过滤**: 每个分身可通过白名单选择使用哪些 MCP 服务器
- **热重载**: 修改配置即时生效

### 记忆层 (Memory) — REMEMBER

跨分身的记忆生命周期管理：

- **ConsolidationEngine** — 每 24h 对 7 天未访问的记忆降低 confidence
- **Session Compaction** — 三阶段 LLM 压缩（>30 条消息触发）
- **Structured KV / Semantic / Knowledge Graph** — 多种记忆存储
- **Canonical Session** — 跨渠道持久会话

---

## Crate 结构

```
opencarrier-types          共享类型 (Agent, Capability, Config, Message, Tool...)
opencarrier-memory         SQLite 记忆层 (KV / Semantic / Knowledge Graph / Session)
opencarrier-runtime        Agent loop + 3 LLM drivers + 23 tools + MCP + A2A
opencarrier-kernel         内核: 组装所有子系统, RBAC, 调度, 触发器
opencarrier-api            REST/WS/SSE API + Dashboard + 引导页 + 出生证
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
| wecom | 企业微信（SmartBot / 应用 / 客服） |
| weixin | 个人微信（iLink 协议，QR 码登录） |
| feishu | 飞书 |
| dingtalk | 钉钉 |
| bilibili | B站 |
| xiaohongshu | 小红书 |
| zhihu | 知乎 |
| twitter | Twitter/X |
| reddit | Reddit |

插件仓库: [opencarrier-plugins](https://github.com/yinnho/opencarrier-plugins)

---

## Security

- WASM 双计量沙箱 — 燃料 + epoch 中断
- Merkle 哈希链审计 — 每个操作密码学链接
- Ed25519 分身签名 — 身份和能力集签名
- SSRF 防护 — 阻断私有 IP、云元数据端点
- Capability Gates — RBAC 能力门控
- Loop Guard — SHA256 工具循环检测 + 熔断器
- Secret Zeroization — API key 自动擦除

---

## Development

```bash
cargo build --workspace --lib          # 编译
cargo test --workspace                 # tests
cargo clippy --workspace --all-targets -- -D warnings  # 0 warnings
```

---

## License

MIT
