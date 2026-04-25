# 分身多实例 + 投喂目录 实现方案

## 需求概述

### A. 多实例克隆
Owner 在 dashboard 新建分身时：输入自定义名字 → 选择 Hub 模板 → 下载安装。
同一个 owner 可以从同一个模板创建多个不同名字的分身实例。

### B. 投喂目录 input/
每个分身 workspace 下自带 `input/` 目录，owner 通过 Web API 上传学习资料。
分身可通过 `file_read("input/xxx")` 读取并学习，学习结果存入 memory，不改 SOUL.md。

### C. 名称 per-tenant 唯一
同 tenant 下分身名字不可重复，不同 tenant 可以有同名分身。
ID (UUID) 是主键，名字是 per-tenant 内的友好标识。

---

## 改动清单

### 1. Registry 名称唯一性改为 per-tenant

**文件**: `crates/opencarrier-kernel/src/registry.rs`

**当前**: `name_index: DashMap<String, AgentId>` — key 是纯名字，全局唯一。

**改为**: `name_index: DashMap<(Option<String>, String), AgentId>` — key 是 `(tenant_id, name)` 组合。

```rust
// name_index key 格式
// (Some("tenant-uuid"), "找车能手") → AgentId
// (None, "assistant")              → AgentId  (系统 agent)
```

**改动点**:
- `register()`: 检查 `(entry.tenant_id, entry.name)` 是否已存在
- `remove()`: 删除时用 `(entry.tenant_id, entry.name)` 移除
- `find_by_name()`: 改为 `find_by_name(name, tenant_id)` — 需要 tenant 上下文
- `update_name()`: 碰撞检查改为 per-tenant
- 所有调用 `find_by_name()` 的地方加 tenant_id 参数

**影响范围**: kernel.rs、hub.rs (API route)、clones.rs、tool_runner.rs 中所有 `find_by_name()` 调用处。

### 2. Hub 安装流程支持自定义名字

**文件**: `crates/opencarrier-api/src/routes/hub.rs`

**当前**: `POST /api/hub/templates/{template_name}/install` — 安装后分身名 = 模板名。

**改为**: request body 增加 `target_name` 字段。

```rust
// 请求体
{
    "target_name": "找车能手",     // owner 起的名字
    "tenant_id": "..."            // 可选，admin 指定
}
```

**改动点**:
- 解析 `target_name`，校验格式（1-64 字符，中文/字母/数字/连字符）
- 传给 `kernel.clone_install(target_name, agx_data, tenant_id)`
- 返回值里的 `name` 用 `target_name`

### 3. Hub Modal UI 加名字输入

**文件**: `crates/opencarrier-api/static/js/pages/agents.js`
**文件**: `crates/opencarrier-api/static/index_body.html`

**改动点**:
- Hub 模板列表每个模板旁增加"分身名称"输入框
- 点安装时校验名字非空
- `installHubTemplate(template_name, target_name)` 传两个字段

**UI 交互**:
```
┌─────────────────────────────────────┐
│ Hub 分身商店                         │
├─────────────────────────────────────┤
│ [customer-service]  客服助手 v1.0    │
│ 提供智能客服...                      │
│ 分身名称: [找车能手          ]       │
│                     [安装]           │
├─────────────────────────────────────┤
│ [writer]  写作助手 v2.1             │
│ 专业写作...                          │
│ 分身名称: [估价能手          ]       │
│                     [安装]           │
└─────────────────────────────────────┘
```

安装成功后 agents 列表显示的是 owner 输入的"找车能手"，不是模板名。

### 4. 投喂目录 input/

#### 4a. 安装时自动创建 input/ 目录

**文件**: `crates/opencarrier-kernel/src/kernel.rs` (`clone_install` 函数)

**改动点**: 在 workspace 创建后，额外创建 `input/` 子目录。

```rust
// 在 install_clone_to_workspace() 之后
std::fs::create_dir_all(workspace_dir.join("input"))?;
```

#### 4b. 投喂上传 API

**文件**: `crates/opencarrier-api/src/routes/clones.rs` (或新文件)

**新增端点**: `POST /api/clones/{agent_id}/input`

```rust
// 请求: multipart/form-data
// - file: 上传的文件
// - path: 可选，目标文件名（默认用原始文件名）

// 响应:
{
    "status": "ok",
    "path": "input/产品手册.pdf",
    "size": 12345
}
```

**权限**: 只有 owner (同 tenant) 可以上传。

#### 4c. 投喂文件列表 API

**新增端点**: `GET /api/clones/{agent_id}/input`

```rust
// 响应:
{
    "files": [
        { "name": "产品手册.pdf", "size": 12345, "modified": "2026-04-25T..." },
        { "name": "价格表.xlsx", "size": 8900, "modified": "2026-04-25T..." }
    ]
}
```

#### 4d. 投喂文件删除 API

**新增端点**: `DELETE /api/clones/{agent_id}/input/{filename}`

#### 4e. file_read 路径处理

**文件**: `crates/opencarrier-runtime/src/tool_runner.rs` (或 sandbox)

**当前**: `file_read("input/xxx")` → 重写为 `users/{sender_id}/input/xxx`。

**改为**: `file_read("input/xxx")` → 直接读 workspace 下的 `input/xxx`，不重写。

- `input/` 前缀的路径：读 `{workspace}/input/xxx`（投喂资料）
- `users/` 前缀的路径：保持现有逻辑（用户文件）
- 其他路径：保持现有逻辑

### 5. 分身自学习（行为层面）

**不改 SOUL.md**，分身通过以下方式学习：

1. 分身的 system_prompt 中加入提示：
   ```
   你可以在 input/ 目录下找到 owner 上传的学习资料。
   当用户提问时，先检查 input/ 目录是否有相关资料，读取并参考。
   ```

2. 分身读取 `input/` 内容后，用 `memory_store` 将关键信息存入记忆。

3. 后续对话中分身可以通过 `memory_recall` 调用已学习的知识。

**不需要新增代码**，只需要在分身生成器 (clone-creator) 的默认 system_prompt 模板中加入上述提示。

---

## 改动规模评估

| 模块 | 改动量 | 说明 |
|------|--------|------|
| registry.rs | 中 | name_index 加 tenant 维度，改 5-6 个方法 |
| hub.rs (API) | 小 | 加 target_name 字段 |
| kernel.rs | 小 | clone_install 创建 input/ 目录 |
| clones.rs | 小 | 新增 3 个端点（上传/列表/删除） |
| agents.js | 小 | Hub modal 加名字输入框 |
| index_body.html | 小 | Hub modal HTML 加输入框 |
| tool_runner.rs | 小 | file_read 的 input/ 路径不重写 |
| 调用方适配 | 小 | find_by_name 加 tenant_id 参数 |

**总计**: 约 8 个文件，核心逻辑改动不大。

---

## 验证清单

- [ ] 同一 tenant 安装两个同名分身 → 第二个被拒绝
- [ ] 不同 tenant 安装同名分身 → 都成功
- [ ] Hub 安装时输入自定义名字 → 分身名显示为自定义名
- [ ] 安装后 workspace 在 `workspaces/{tenant_uuid}/{自定义名}/` 下
- [ ] workspace 下有 `input/` 空目录
- [ ] Owner 上传文件到 input/ → 成功
- [ ] 非同 tenant 用户上传 → 被拒绝
- [ ] 分身 `file_read("input/xxx")` → 正确读取投喂文件
- [ ] 分身读取后用 memory_store 存储 → 后续可 recall
- [ ] Agent 列表、聊天头部显示的是 owner 输入的名字
