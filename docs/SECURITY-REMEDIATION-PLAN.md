# 安全整改方案：多租户隔离补全

## 背景

系统已有 `can_access()` 做 API 层的租户隔离，但工具层（agent 通过 tool 调用）
和部分 API 端点完全没有租户校验。一个分身可以通过工具读写其他租户的文件、
给其他租户的分身发消息。

## 发现清单

### CRITICAL-1: train_* 工具无租户隔离

**文件**: `tool_runner.rs:1824-1840`, `kernel.rs:4783-4788`

`resolve_target_workspace()` 按**名字**全局查找目标分身，不检查 caller 和 target
是否同 tenant。clone-trainer 可以读写任何租户的分身文件。

**调用链**:
```
clone-trainer → train_read(target="别的租户的分身名")
→ resolve_target_workspace(input, kernel)
→ kernel.resolve_agent_workspace(target)     // 无 tenant 参数
→ registry.find_by_name(target)              // 全局查找
→ 返回目标 workspace 路径                     // 越权成功
```

**影响**: 10 个 train_* 工具全部受影响：train_read, train_write, train_list,
train_knowledge_add/import/list/read/lint/heal, train_evaluate。

**整改**:
- `resolve_target_workspace` 增加 `caller_agent_id` 参数
- 通过 `kernel.get_agent_tenant_id(caller_id)` 获取 caller 的 tenant
- 通过 `find_by_name(target)` 获取 target entry，检查 target.tenant_id == caller.tenant_id
- 不匹配则返回错误
- 所有 train_* 工具函数签名改为接收 `caller_agent_id`

### CRITICAL-2: registry.find_by_name 全局查找

**文件**: `registry.rs:53-66`

`find_by_name()` 返回全局第一个匹配。改成 per-tenant 唯一后，不同 tenant 有同名分身，
这个函数会返回错误的那个。线性扫描兜底同样有问题。

**整改**:
- 新增 `find_by_name_and_tenant(name, tenant_id)` 方法
- `name_index` 的 key 改为 `(Option<String>, String)` 即 `(tenant_id, name)`
- `register()` / `remove()` / `find_by_name()` / `update_name()` 全部适配
- 保留 `find_by_name()` 作为全局查找（内部使用），但所有跨租户场景必须用新方法

### HIGH-1: POST /hooks/agent 无租户校验

**文件**: `api/routes/webhooks.rs:109-135`

webhook 端点只验证 bearer token，之后按名字找 agent 并发消息，不检查租户归属。

**整改**:
- webhook 需要关联 tenant_id（webhook token 绑定 tenant）
- 解析 agent 后检查 `can_access(ctx, entry.tenant_id)`

### HIGH-2: GET /api/agents/{id}/sessions/by-label/{label} 无租户上下文

**文件**: `api/routes/sessions.rs:270-308`

该端点没有 `extensions` 参数，无法提取 TenantContext。

**整改**:
- 函数签名加上 `extensions: axum::http::Extensions`
- 用 `get_tenant_ctx(&extensions)` 提取上下文
- 解析 agent 后检查 `can_access`

### HIGH-3: send_to_agent 无租户校验

**文件**: `kernel.rs:4458-4465`

`agent_send` 工具调用 `kernel.send_to_agent()`，按名字查找目标 agent，
不检查 caller 和 target 是否同 tenant。

**整改**:
- `send_to_agent()` 增加 `caller_tenant_id` 参数
- 查找 target 后校验 `target.tenant_id == caller_tenant_id`（admin 除外）
- `agent_send` 工具传入 caller 的 tenant_id

### MEDIUM-1: clone_install / spawn 全局重名检查

**文件**: `kernel.rs:4840`, `registry.rs:32-34`

名字唯一性是全局的。Tenant A 装了 "support"，Tenant B 就不能装同名的。
而且还可能被用来 DoS（抢注常见名字）。

**整改**:
- 配合 CRITICAL-2 的 `name_index` 改造
- `register()` 改为 per-tenant 唯一
- `clone_install` 的碰撞检查改为同 tenant 内查重

### LOW-1: caller_agent_id.unwrap_or("default")

**文件**: `tool_runner.rs:93, 3773, 3817, 3894`

agent_loop 总是传 `Some(&caller_id_str)`，正常情况不会触发。
但如果新增调用路径忘了传，browser/docker/process 命名空间会冲突。

**整改**:
- 改为 `.ok_or("Missing caller_agent_id")?` — 快速失败
- 不做兜底

## 改动文件

| 文件 | 改动 |
|------|------|
| `crates/opencarrier-kernel/src/registry.rs` | name_index 改 per-tenant，新增 find_by_name_and_tenant |
| `crates/opencarrier-runtime/src/tool_runner.rs` | train_* 加 tenant 校验，unwrap_or("default") 改报错 |
| `crates/opencarrier-kernel/src/kernel.rs` | send_to_agent/resolve_agent_workspace 加 tenant 参数 |
| `crates/opencarrier-runtime/src/kernel_handle.rs` | trait 方法签名更新 |
| `crates/opencarrier-api/src/routes/webhooks.rs` | 加 tenant 校验 |
| `crates/opencarrier-api/src/routes/sessions.rs` | 加 extensions 参数和 can_access |

## 优先级

1. **先做 CRITICAL-1 + CRITICAL-2** — train_* 工具越权 + registry 改造，这是核心安全问题
2. **再做 HIGH-1/2/3** — API 端点和 send_to_agent
3. **最后做 MEDIUM-1 + LOW-1** — 功能完善和防御性编程

## 验证

- [ ] Tenant A 的 clone-trainer 不能读写 Tenant B 的分身文件
- [ ] 同名分身可以在不同 tenant 下共存
- [ ] webhook 只能发给自己 tenant 的 agent
- [ ] session 查询受 tenant 限制
- [ ] agent_send 不能跨 tenant 发消息（admin 除外）
- [ ] `cargo build --workspace --lib` 编译通过
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 无警告
