# 安全整改方案：多租户隔离补全

## 背景

系统已有 `can_access()` 做 API 层的租户隔离，但工具层（agent 通过 tool 调用）
和部分 API 端点完全没有租户校验。一个分身可以通过工具读写其他租户的文件、
给其他租户的分身发消息。

## 已修复清单

### CRITICAL-1: train_* 工具无租户隔离 ✅

`resolve_target_workspace` 使用 `resolve_agent_workspace_in_tenant` 做单次 tenant-scoped 查找。

### CRITICAL-2: registry.find_by_name 全局查找 ✅

`name_index` key 改为 `(tenant_id, name)`。新增 `find_by_name_and_tenant`。

### HIGH-1: POST /hooks/agent ✅

`AgentHookPayload` 新增 `tenant_id` 字段。名字查找必须提供 tenant_id。

### HIGH-2: sessions/by-label ✅

添加 `extensions` 参数，使用 `resolve_agent_id_with_tenant`。

### HIGH-3: send_to_agent ✅

trait 新增 `caller_agent_id`。名字查找必须通过 caller 的 tenant 做 scoped lookup，无 caller 时拒绝名字查找。

### HIGH-4: delete_session / set_session_label ✅

添加 `extensions`，通过 session→agent→tenant 做 `can_access` 校验。

### HIGH-5: clone_agent 不给 clone 分配 tenant_id ✅

spawn 后调用 `registry.set_tenant_id(new_id, ctx.tenant_id)`。

### HIGH-6: train_* 基础查找 ✅

新增 `resolve_agent_workspace_in_tenant(name, tenant_id)`。

### HIGH-7: mcp_http 无 tenant/sender 上下文 ✅

添加 `extensions` 参数 + tenant 认证。拦截需要 agent 上下文的工具（agent_send/spawn/list、train_*、memory、task 等）。

### HIGH-8: list_agents KernelHandle 返回全量 ✅

`list_agents` trait 方法新增 `caller_tenant_id` 参数。`tool_agent_list` 传入 caller 的 tenant 过滤。

### MEDIUM-1: clone 端点全局 find_by_name ✅

install/start/stop/uninstall 全部改为 tenant-scoped 查找。

### MEDIUM-2: clone_install kernel 碰撞 ✅

改为 `find_by_name_and_tenant` 碰撞检查。

### MEDIUM-3: tenant 创建 auto-start ✅

改为 `find_by_name_and_tenant(clone_name, Some(&tenant_id))`。

### MEDIUM-4: add_binding 无租户校验 ✅

list/add/remove binding 全部加了 tenant 校验。非 admin 只能操作自己 agent 的 binding。

### LOW-1: unwrap_or("default") ✅

全部改为 `.ok_or("Missing caller agent identity")?`。

## 改动文件汇总

| 文件 | 改动 |
|------|------|
| `registry.rs` | name_index per-tenant，新增 find_by_name_and_tenant |
| `tool_runner.rs` | train_* tenant 校验，agent_list/agent_send tenant 过滤 |
| `kernel.rs` | send_to_agent/resolve_agent_workspace/list_agents/clone_install tenant 参数 |
| `kernel_handle.rs` | trait 方法签名更新（send_to_agent/list_agents/resolve_agent_workspace_in_tenant） |
| `webhooks.rs` | 名字查找必须提供 tenant_id |
| `sessions.rs` | delete/set_label/by-label 全部加 tenant 校验 |
| `agents.rs` | clone_agent 正确继承 tenant_id |
| `clones.rs` | install/start/stop/uninstall tenant-scoped |
| `common.rs` | 新增 resolve_agent_id_with_tenant，get_clone_workspace_with_tenant 改为 scoped |
| `tools_skills.rs` | mcp_http 加 tenant 认证 + 工具拦截 |
| `bindings.rs` | list/add/remove 全部加 tenant 校验 |
| `tenants.rs` | auto-start 改为 tenant-scoped 查找 |
| `host_functions.rs` | send_to_agent 签名更新 |
| `plugin/bridge.rs` | send_to_agent 签名更新 |
| `plugin/manager.rs` | list_agents(None) |
| `webhook.rs` (types) | AgentHookPayload 新增 tenant_id |

## 验证

- [x] `cargo build --workspace --lib` 编译通过
- [x] `cargo clippy --workspace --all-targets -- -D warnings` 无警告
- [x] `cargo test --workspace` 全部通过
- [x] Tenant A 的 clone-trainer 不能读写 Tenant B 的分身文件
- [x] 同名分身可以在不同 tenant 下共存
- [x] agent_list 只能看到同 tenant 的 agent
- [x] agent_send 不能跨 tenant 发消息
- [x] session 删除/标签修改受 tenant 限制
- [x] clone 安装/启动/停止/卸载 tenant-scoped
- [x] binding 操作受 tenant 限制
- [x] mcp_http 拦截跨租户工具
- [x] webhook 名字查找必须提供 tenant_id
