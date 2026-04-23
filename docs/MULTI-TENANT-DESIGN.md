# Multi-Tenant Design — OpenCarrier SaaS

## Overview

OpenCarrier 从单实例单租户改为单实例多租户，一个 opencarrier 进程服务多个企业（租户）。

核心思路：**全局共享 + tenant_id 过滤**。

## 租户模型

| 角色 | 权限 |
|------|------|
| **Admin** | 管理全局配置（.env、brain.json、plugins）、创建/管理租户、查看所有数据 |
| **Tenant** | 管自己的分身（clone）、定时任务（cron）、通道绑定（channel binding）|

租户不能访问：`.env`、`brain.json`、`config.toml`、全局插件管理。

## 数据隔离

### 共享（全局，admin 管理）

- `.env` — 所有 API keys
- `brain.json` — LLM 路由
- `config.toml` — 全局配置
- `plugins/` — 插件二进制
- 二进制本身

### 按租户隔离

- agents — 分身归属
- sessions / memory — 对话记忆
- cron_jobs — 定时任务
- channel_bindings — 通道配置
- workspaces/ — 分身文件目录
- usage — 用量计费

## DB Schema Changes

### 新增表

```sql
-- 租户表（兼作用户表）
CREATE TABLE tenants (
    id          TEXT PRIMARY KEY,      -- UUID
    name        TEXT NOT NULL UNIQUE,  -- 租户名/登录名
    password_hash TEXT NOT NULL,
    api_key     TEXT,                  -- 租户级 API key（可选）
    role        TEXT NOT NULL DEFAULT 'tenant',  -- 'admin' | 'tenant'
    status      TEXT NOT NULL DEFAULT 'active',  -- 'active' | 'suspended'
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

-- 租户通道绑定
CREATE TABLE tenant_channels (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL REFERENCES tenants(id),
    channel_type TEXT NOT NULL,         -- 'wecom_smartbot', 'wecom_kf', etc.
    config_json TEXT NOT NULL,          -- {"corp_id":"...","bot_id":"...","secret_env":"..."}
    bind_agent_id TEXT,                 -- agent UUID，绑定到的分身
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
```

### 现有表加 tenant_id

所有数据表加 `tenant_id TEXT NOT NULL`：

| 表 | 说明 |
|----|------|
| `agents` | 分身归属 |
| `sessions` | 对话 |
| `events` | 事件 |
| `kv_store` | KV 存储 |
| `task_queue` | 任务队列 |
| `memories` | 语义记忆 |
| `entities` | 知识图谱实体 |
| `relations` | 知识图谱关系 |
| `usage_events` | 用量 |
| `canonical_sessions` | 跨通道记忆 |
| `audit_entries` | 审计 |
| `kv_history` | KV 历史 |

Migration: v10 — `ALTER TABLE ... ADD COLUMN tenant_id TEXT NOT NULL DEFAULT ''`
之后通过数据迁移脚本填充（admin 默认给一个 tenant_id）。

### 索引

```sql
CREATE INDEX idx_agents_tenant ON agents(tenant_id);
CREATE INDEX idx_sessions_tenant ON sessions(tenant_id);
-- ... 其他表类似
```

## Auth System

### 登录

```
POST /api/auth/login { username, password }
→ 查 tenants 表（name = username）
→ 验证 password_hash
→ 返回 session token 含 { tenant_id, role, username }
```

### Session Token

```
payload: { tenant_id, role, username, expiry }
HMAC 签名
```

### Middleware

```
每个请求：
1. 从 session token 或 Bearer token 解出 tenant_id + role
2. 注入 request extensions
3. handler 通过 Extension<TenantContext> 获取
```

```rust
struct TenantContext {
    tenant_id: String,
    role: String,  // "admin" | "tenant"
}
```

### API Key

- Admin api_key: `config.toml` 里的全局 key（保持现有行为）
- Tenant api_key: `tenants` 表里的 key，Bearer 认证时匹配到对应租户

## API 层

### Admin 端点（需要 role=admin）

```
POST   /api/admin/tenants              创建租户
GET    /api/admin/tenants              租户列表
PUT    /api/admin/tenants/{id}         修改租户
DELETE /api/admin/tenants/{id}         删除租户
POST   /api/admin/tenants/{id}/reset-key  重置 API key
```

### 租户端点（自动按 tenant_id 过滤）

所有现有 `/api/agents`, `/api/clones`, `/api/sessions`, `/api/cron/jobs`, `/api/usage` 等：
- GET: 加 `WHERE tenant_id = ?`
- POST: 自动填入当前租户的 tenant_id
- PUT/DELETE: 验证资源属于当前租户

### 全局端点（不受租户限制）

```
GET /api/health          健康检查
GET /api/status          服务状态
GET /api/plugins         插件列表（admin 功能）
GET /api/config          全局配置（admin only）
GET /api/brain/*         Brain 配置（admin only）
```

## File System

```
~/.opencarrier/
├── config.toml              全局
├── brain.json               全局
├── .env                     全局
├── plugins/                 全局
├── data/
│   └── opencarrier.db       共享 DB（tenant_id 过滤）
└── tenants/
    ├── {tenant_id}/
    │   ├── workspaces/      租户的分身工作空间
    │   │   ├── car-finder-v2/
    │   │   └── ...
    │   ├── sessions/        租户的会话文件
    │   └── cron_jobs.json   租户的定时任务
    └── ...
```

### 路径变更

| 当前 | 改为 |
|------|------|
| `workspaces/{agent_name}/` | `tenants/{tenant_id}/workspaces/{agent_name}/` |
| `cron_jobs.json` | `tenants/{tenant_id}/cron_jobs.json` |
| `sessions/` | `tenants/{tenant_id}/sessions/` |

## Plugin System

### 通道绑定

当前：`plugin.toml` 静态配置 `bind_agent`
改为：`tenant_channels` 表动态配置

### Bridge Routing

```
消息进来 → PluginMessage { tenant_id, sender_id, channel_type, ... }
→ bridge 查 tenant_channels 表
→ 找到 bind_agent_id（且该 agent 属于同一 tenant）
→ 路由到对应 agent
```

### 租户自己配置通道

```
POST /api/channels/bind
{
    "channel_type": "wecom_smartbot",
    "config": { "corp_id": "...", "bot_id": "...", "secret_env": "WECOM_BOT_SECRET" },
    "bind_agent_id": "agent-uuid"
}
```

租户只能绑定自己的 agent。

## Cron Jobs

`CronJob` 结构体加 `tenant_id`。

`CronScheduler` 按 tenant_id 过滤：
- `list_jobs()` → `WHERE tenant_id = ?`
- `create_job()` → 自动填 tenant_id
- `due_jobs()` → 执行时验证 agent 归属

## Usage / Billing

`usage_events` 表加 `tenant_id`。

```
GET /api/usage          → 租户只能看自己的
GET /api/usage/summary  → 租户自己的汇总
Admin 可以看所有租户的用量
```

## 实施步骤

1. **DB migration** — 新增 tenants、tenant_channels 表，现有表加 tenant_id
2. **Auth** — 登录走 tenants 表，session token 含 tenant_id
3. **Middleware** — 注入 TenantContext
4. **API routes** — 所有端点加 tenant 过滤
5. **File paths** — workspaces 按 tenant 隔离
6. **Plugin bridge** — 动态路由
7. **Cron** — 按 tenant 隔离
8. **Frontend** — admin 看到"租户管理"tab，tenant 只看到自己的

## 向后兼容

- 现有单租户部署自动升级：admin 账号从 config.toml 的 username/password 迁移到 tenants 表
- 没有 tenant_id 的数据默认归 admin
- 不配置多租户时行为不变
