# OpenCarrier + Aginx 系统架构

> **版本**: v1.0
> **日期**: 2026-04-23

---

## 一句话

**一个 OpenCarrier 进程 = N 个独立 agent 对外服务，通过 aginx 网关暴露到互联网，用 agc 工具任意访问。**

---

## 核心模型

```
互联网
  │
  │  agc / 任何 ACP 客户端
  │  agent://abc123.relay.aginx.net/客服助手
  │
  ▼
┌─────────────────────────────────┐
│  aginx（Agent 网关 = nginx）      │
│  路由外部请求到对应 agent          │
│  agent:// 地址 → agent UUID       │
└──────────────┬──────────────────┘
               │ ACP / JSON-RPC over stdio
               ▼
┌─────────────────────────────────┐
│  OpenCarrier（Agent 托管引擎）    │
│                                 │
│  ┌──────────┐ ┌──────────┐      │
│  │ Agent A  │ │ Agent B  │ ...  │
│  │ UUID-aaa │ │ UUID-bbb │      │
│  │ 公司甲    │ │ 公司乙   │      │
│  │ 客服助手  │ │ 销售顾问  │      │
│  └──────────┘ └──────────┘      │
│                                 │
│  每个 Agent 独立的：              │
│  · workspace（SOUL.md、知识库）  │
│  · session（对话历史）            │
│  · KV 记忆（按 agent UUID）      │
│  · 定时任务                      │
│  · 技能和工具                    │
└─────────────────────────────────┘
```

---

## 身份模型：Agent UUID = 门牌号

**Agent UUID 是系统中唯一的身份标识，内外统一。**

| 场景 | UUID 的作用 |
|------|------------|
| 内部数据隔离 | KV 记忆的命名空间：`{agent_uuid} → {key} → {value}` |
| 对话管理 | session 按 agent_id 绑定 |
| 工作空间 | `tenants/{tenant_id}/workspaces/{agent_name}/` |
| 外部寻址 | `agent://abc123.relay.aginx.net/{agent_name}` |
| Agent 互联 | aginx 通过 UUID 路由到正确的 agent |
| 定时任务 | cron job 绑定到具体 agent |

同一个分身模板（如"客服助手"），被不同公司安装后，生成不同的 Agent UUID，数据完全隔离：

```
"客服助手" 模板
  ├── 公司甲安装 → Agent UUID-aaa → 独立记忆/会话/知识库
  └── 公司乙安装 → Agent UUID-bbb → 独立记忆/会话/知识库
```

---

## 三层架构

| 层 | 组件 | 类比 | 职责 |
|----|------|------|------|
| **网关层** | aginx | nginx | 路由、认证、TLS、多 Agent 管理 |
| **引擎层** | OpenCarrier | 应用服务器 | Agent 生命周期、LLM 调用、记忆、工具 |
| **客户端** | agc / aginxium | curl / Chrome | 发起请求、展示结果 |

### aginx（网关层）

- **纯消息路由**，不关心 Agent 内部用什么模型、什么框架
- 一个 aginx 实例可以管理多个 OpenCarrier 进程（或其他 ACP Agent）
- 通过 `aginx.toml` 配置 Agent 列表
- 支持 relay 中继（NAT 穿透）和 TCP 直连
- 认证：设备绑定 token 或 JWT（aginx-api 签发）

```
$ aginx

========================================
aginx v0.2.2
Agent 地址: agent://abc123.relay.aginx.net
已注册 Agent: 3
  - 客服助手    OpenCarrier (chat, tools)
  - 销售顾问    OpenCarrier (chat, tools)
  - copilot     GitHub Copilot (code, ask)
========================================
```

### OpenCarrier（引擎层）

- **Agent 托管平台**，一个进程跑 N 个独立 agent
- 每个 agent 拥有完整的数据体系：人格（SOUL.md）、指令、知识库、技能、记忆
- 多租户隔离：不同租户的 agent 数据互不可见
- 内置 ACP 协议支持（`opencarrier serve` 模式），可直接接入 aginx

### agc（客户端工具）

`curl` for `agent://` — 一行命令访问任何 agent：

```bash
# 访问公司甲的客服助手
agc agent://abc123.relay.aginx.net/客服助手 "查一下订单 #12345"

# 访问另一个 aginx 上的 copilot
agc agent://xyz789.relay.aginx.net/copilot "fix the bug"

# Agent 之间也可以互相调用
# Agent A 在处理任务时，可以通过 aginx 调用 Agent B
```

---

## 数据隔离

### 每个 Agent 独立拥有

| 数据 | 存储方式 | 隔离边界 |
|------|---------|---------|
| 人格/指令 | workspace 文件（SOUL.md、system_prompt.md） | agent UUID |
| 知识库 | workspace/knowledge/*.md | agent UUID |
| 对话历史 | session（SQLite，按 agent_id） | agent UUID |
| KV 记忆 | structured KV（SQLite，按 agent_id） | agent UUID |
| 语义记忆 | semantic memory（SQLite，按 agent_id） | agent UUID |
| 定时任务 | cron jobs（按 agent_id 绑定） | agent UUID |
| 技能/工具 | workspace/skills/*.md | agent UUID |

### 全局共享（Admin 管理）

| 资源 | 说明 |
|------|------|
| API Keys | 所有 agent 共用（BYOK） |
| LLM 路由 | brain.json — 模型/供应商配置 |
| 插件 | 二进制文件 |
| 配置 | config.toml |
| 进程本身 | 一个 OpenCarrier 二进制 |

### KV 记忆模型

每个 agent 的 KV 记忆按 agent UUID 隔离，类似 Redis 的命名空间：

```
Agent UUID-aaa:
  ├── "退货政策" → "7天无理由退货..."
  ├── "常用话术" → "您好，很高兴为您服务..."
  └── "user_name" → "张三"

Agent UUID-bbb:
  ├── "退货政策" → "30天退货保障..."   # 同名 key，不同值
  └── "常用话术" → "Hello, how can I help..."
```

不存在"全局共享记忆"。每个 agent 就是一个天然的命名空间，外部通过 agent UUID 访问该 agent 的记忆。

---

## 多租户模型

```
OpenCarrier 进程
├── Tenant 甲（公司甲）
│   ├── Agent UUID-aaa（客服助手）  ← aginx 路由到这里
│   └── Agent UUID-bbb（销售顾问）  ← aginx 路由到这里
│
├── Tenant 乙（公司乙）
│   ├── Agent UUID-ccc（客服助手）  ← 同模板，不同实例，完全隔离
│   └── Agent UUID-ddd（技术支持）  ← aginx 路由到这里
│
└── Admin
    └── 管理全局配置、LLM 路由、API Keys
```

| 角色 | 权限 |
|------|------|
| **Admin** | 全局配置、租户管理、查看所有数据 |
| **Tenant** | 管理自己的 agent、会话、定时任务、通道绑定 |

租户不能访问：API Keys、LLM 配置、全局插件、其他租户的数据。

---

## 通信协议：ACP

所有组件通过 **ACP（Agent Client Protocol）** 通信，基于 JSON-RPC 2.0：

```
客户端（agc/aginxium/App）
  │
  │  ACP (JSON-RPC 2.0, ndjson)
  │  agent:// 地址
  │
  ▼
aginx（网关）
  │
  │  ACP (stdio/TCP)
  │  路由到具体 agent
  │
  ▼
OpenCarrier agent
```

OpenCarrier 的 `serve` 模式实现了 ACP server 端，可以直接作为 aginx 的后端 Agent：

```toml
# aginx.toml — OpenCarrier 作为 aginx 的后端 agent
[[agents]]
id = "opencarrier"
name = "OpenCarrier"
protocol = "acp"
command = "opencarrier"
args = ["serve"]
```

---

## Agent 互联

这是系统的关键能力——Agent 可以访问 Agent：

```
公司甲的客服助手
  │
  │  发现公司甲的销售顾问有客户购买记录
  │  通过 aginx + agc 调用
  │
  ▼
公司甲的销售顾问 → 返回购买记录
```

```
本地的客服助手
  │
  │  需要翻译一段多语言内容
  │  通过 aginx relay 调用远程的翻译 Agent
  │
  ▼
远程翻译 Agent → 返回翻译结果
```

aginx 的 relay 网络让 Agent 可以跨实例、跨网络互相访问。任何实现了 ACP 的 Agent 都可以接入——OpenCarrier agent、GitHub Copilot、Gemini CLI 等。

---

## 两条访问路径，两套隔离机制

OpenCarrier 有两条被访问的路径，隔离机制不同：

### 路径 1：ACP + aginx（外部访问）

```
外部客户端 → aginx → OpenCarrier（ACP stdio）
```

- **隔离由 aginx 负责**：aginx.toml 配置每个 agent 的 ID 和归属
- **agent 名字可以重名**：不同 aginx 实例上的"客服助手"是不同 agent（不同 ID）
- **OpenCarrier ACP 层不需要知道 tenant**：请求到达时，agent 已经被 aginx 路由好了
- **必须指定 agent**：没有默认行为，不传 agentId 直接报错

### 路径 2：HTTP API（内部管理 / SaaS Dashboard）

```
浏览器 Dashboard → HTTP API → OpenCarrier
```

- **隔离由 OpenCarrier 的 tenant_id 系统负责**：TenantContext 从 auth middleware 注入
- **多租户共享一个进程**：不同租户通过 tenant_id 过滤看到自己的 agent
- **Admin 看到全部**：admin 角色不受租户过滤

### KV 记忆的隔离

| 访问路径 | KV 隔离方式 |
|---------|-----------|
| ACP（外部） | 按 agent UUID，天然隔离 |
| HTTP API（内部） | 按 agent UUID + tenant_id 校验 |

每个 agent 的 KV 记忆按 agent UUID 隔离，类似 Redis 的命名空间。不存在"全局共享记忆"。

---

## 部署拓扑

### 小规模（单机）

```
一台服务器
├── aginx       ← 网关，对外暴露
├── OpenCarrier  ← 引擎，跑 N 个 agent
└── aginx.toml  ← 配置 agent 列表
```

外部通过 `agent://abc123.relay.aginx.net/客服助手` 直接访问。

### 中规模（多机）

```
服务器 A
├── aginx + OpenCarrier
└── 客服助手、销售顾问

服务器 B
├── aginx + OpenCarrier
└── 技术支持、数据分析师

aginx-api（注册中心）
└── 记录所有 aginx 实例和 agent 目录

aginx-relay（中继）
└── NAT 穿透，让任意两个 agent 互联
```

### 大规模（集群）

```
aginx-api    ← 注册中心，DNS 角色
aginx-relay  ← 中继网络，CDN 角色
aginx × N    ← 每台机器一个，路由层
OpenCarrier × M  ← 引擎层，按负载水平扩展
```

---

## 设计原则

1. **Agent UUID 是一等公民** — 内外统一，寻址、隔离、认证都基于它
2. **aginx 不关心 Agent 内部** — 纯路由，就像 nginx 不关心网站用什么语言
3. **数据跟着 Agent 走** — 每个 Agent 自带完整数据，不依赖全局共享
4. **OpenCarrier 是引擎不是框架** — 一个二进制跑起来，管理 N 个 Agent
5. **ACP 是开放协议** — 任何 Agent 都能接入，不绑定 OpenCarrier

---

## 相关文档

| 文档 | 位置 | 说明 |
|------|------|------|
| Aginx 架构 | `/docs/aginx/aginx/CLAUDE.md` | aginx 网关开发文档 |
| ACP 协议 | `/docs/aginx/ACP.md` | ACP 协议完整规范 |
| Aginx 远景 | `/docs/aginx/VISION.md` | Agent 互联网愿景 |
| agent:// 协议 | `/docs/aginx/docs/agent-protocol.md` | 通信协议细节 |
| agc 工具 | `/docs/aginx/agc/README.md` | agent:// 的 curl |
| OpenCarrier 架构 | `docs/ARCHITECTURE-PRINCIPLES.md` | 分身 OS 架构原则 |
| 多租户设计 | `docs/MULTI-TENANT-DESIGN.md` | 租户隔离详细设计 |
