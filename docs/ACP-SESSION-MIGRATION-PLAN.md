# OpenCarrier ACP 会话存储统一迁移计划（最终版）

> 目标：OpenCarrier `serve` 模式彻底统一为 ACP 协议，移除 yinghe/legacy 兼容层，将会话存储从 SQLite+内存混合方案改为 Claude 兼容 JSONL 文件存储，使 aginx 可直接读取。

## 当前架构问题

| 路径 | 当前存储 | 问题 |
|------|---------|------|
| ACP (`acp.rs`) | 纯内存 (`AcpConnectionState.sessions`) | 进程关闭会话丢失，`session/list` 看不到历史，`listConversations` 只能返回当前连接会话 |
| yingheclient (`handle_yingheclient_message`) | SQLite (`yinghe_sessions.db`) | 含有 yinghe 旧标识，协议方向已废弃 |
| legacy JSON-RPC (`hello`/`sendMessage` 等) | 无持久化 | 非 ACP 标准，应移除 |

## 统一后的架构

OpenCarrier serve 模式**只保留 ACP 协议**，所有会话持久化到 JSONL 文件。

### 存储结构

```
~/.opencarrier/sessions/
  {workspace-basename}/
    {session_id}.jsonl       # Claude 兼容格式消息历史
    {session_id}.meta.json   # 元数据（title、cwd、时间）
```

- `workspace-basename` 来自 `session/new` 的 `cwd` 参数（如 `cwd="/project/foo"` → `foo`）
- 与 Claude CLI 的 `~/.claude/projects/{name}/{session_id}.jsonl` 结构一致
- aginx `scan_claude_sessions` 可直接递归扫描

### .jsonl 格式（aginx 兼容）

```jsonl
{"type":"user","timestamp":"2026-04-15T10:00:00Z","cwd":"/project/foo","message":{"content":"Hello"}}
{"type":"assistant","timestamp":"2026-04-15T10:00:01Z","message":{"content":[{"type":"text","text":"Hi!"}]}}
```

### .meta.json 格式

```json
{
  "sessionId": "sess_abc123",
  "agentId": "agent-uuid",
  "cwd": "/project/foo",
  "title": "Hello",
  "createdAt": 1713168000000,
  "updatedAt": 1713168000000
}
```

## 文件变更清单

### 1. NEW: `crates/opencarrier-memory/src/acp_session.rs`

`AcpSessionStore` 实现：

```rust
#[derive(Clone)]
pub struct AcpSessionStore {
    base_dir: PathBuf,  // ~/.opencarrier/sessions
}

impl AcpSessionStore {
    pub fn new(base_dir: &Path) -> Self;

    // 会话生命周期
    pub fn create_session(&self, session_id: &str, agent_id: &str, cwd: &str) -> Result<(), String>;
    pub fn delete_session(&self, session_id: &str) -> Result<bool, String>;
    pub fn list_sessions(&self) -> Result<Vec<serde_json::Value>, String>;
    pub fn session_exists(&self, session_id: &str) -> bool;

    // 消息追加
    pub fn append_user_message(&self, session_id: &str, content: &str, cwd: Option<&str>) -> Result<(), String>;
    pub fn append_assistant_message(&self, session_id: &str, content: &str) -> Result<(), String>;

    // 消息读取（供 _aginx/getMessages 使用）
    pub fn get_messages(&self, session_id: &str, limit: usize) -> Result<Vec<serde_json::Value>, String>;
}
```

并发控制：
- 每个 `.jsonl` 文件使用 `std::sync::Mutex` 按 session_id 粒度锁
- `.meta.json` 使用原子写（`write to temp → rename`）

### 2. EDIT: `crates/opencarrier-memory/src/lib.rs`

```diff
  pub mod session;
  pub mod structured;
  pub mod usage;
- pub mod yinghe_session;
+ pub mod acp_session;
```

### 3. DELETE: `crates/opencarrier-memory/src/yinghe_session.rs`

完全删除。

### 4. EDIT: `crates/opencarrier-cli/src/serve.rs`

**移除内容**：
- `YingheSessionManager` 相关 import 和 `init_session_manager`
- `handle_yingheclient_message` 函数
- `is_yingheclient_format` 函数
- `handle_request` 中的 legacy 方法路由（`hello`、`sendMessage`、`getAgentCard`、`compactMemory`、`bye` 改为返回 `METHOD_NOT_FOUND` 或直接移除）
- `yinghe_sessions.db` 创建逻辑

**新增内容**：
- `use opencarrier_memory::acp_session::AcpSessionStore;`
- `init_acp_session_store` 函数，返回 `AcpSessionStore`
- `run_serve_mode` 中创建 `AcpSessionStore` 并传给 `acp::handle_acp_request`

**消息分发**：
- yingheclient 格式输入直接按 ACP 路由处理，或返回不支持提示
- 由于统一为 ACP，输入检测只保留：是 JSON-RPC 且 `is_acp_method` → ACP；否则返回 `METHOD_NOT_FOUND`

### 5. EDIT: `crates/opencarrier-cli/src/acp.rs`

**接口改动**：
- `handle_acp_request` 新增参数 `store: &AcpSessionStore`

**`acp_session_new`**：
- 调用 `store.create_session(&session_id, agent_id_str, cwd)` 持久化
- 同时保留内存 `state.sessions`（供 prompt 时快速查 agent_id/cwd）

**`acp_session_prompt`**：
- 发送消息前：`store.append_user_message(session_id, &message, Some(&session.cwd))`
- streaming 结束后：收集完整 assistant response，调用 `store.append_assistant_message(session_id, &response_text)`
- 同时更新 `.meta.json` 的 `title`（取 user message 前 100 字符）和 `updatedAt`

**`acp_session_list`**：
- 改为 `store.list_sessions()`，返回 ACP 标准格式：
  ```json
  {"sessions": [{"sessionId": "...", "cwd": "...", "title": "...", "updatedAt": "..."}], "nextCursor": null}
  ```

**`_aginx/listConversations`**：
- 调用 `store.list_sessions()`，字段映射为：
  ```json
  {"conversations": [{"id": "...", "title": "...", "createdAt": ..., "updatedAt": ...}]}
  ```

**`_aginx/getMessages`**：
- 调用 `store.get_messages(session_id, limit)`，返回：
  ```json
  {"messages": [{"role": "user", "content": "..."}, {"role": "assistant", "content": "..."}]}
  ```
- **不再**读取 `kernel.memory.get_session`

**`_aginx/deleteConversation`**：
- 调用 `store.delete_session(session_id)`
- 同时 `state.sessions.remove(session_id)`

**暂不支持**：
- `session/load`：返回 `METHOD_NOT_FOUND "not yet supported"`
- `session/cancel`：返回 `METHOD_NOT_FOUND "not yet supported"`
- `session/request_permission`：返回 `METHOD_NOT_FOUND`

### 6. EDIT: `~/.aginx/agents/opencarrier/aginx.toml`

```toml
id = "opencarrier"
name = "OpenCarrier"
agent_type = "process"
protocol = "acp"
command = "opencarrier"
args = ["serve"]

storage_path = "~/.opencarrier/sessions"
```

## 数据流验证

### ACP 完整流程

```
aginx → initialize
  ← ok

aginx → session/new {cwd: "/project/foo"}
  → acp.rs: acp_session_new
    → acp_session.rs: create_session
      → write ~/.opencarrier/sessions/foo/sess_abc.jsonl (empty)
      → write ~/.opencarrier/sessions/foo/sess_abc.meta.json
  ← {sessionId: "sess_abc"}

aginx → session/prompt {sessionId: "sess_abc", prompt: [...]}
  → acp.rs: acp_session_prompt
    → append_user_message → append JSONL
    → kernel: send_message_streaming
    ← streaming session/update notifications
    → append_assistant_message → append JSONL
    → update meta title
  ← {stopReason: "end_turn"}

aginx → _aginx/listConversations
  → scan ~/.opencarrier/sessions/*/*.meta.json
  ← return conversations

aginx → _aginx/getMessages {conversationId: "sess_abc"}
  → read ~/.opencarrier/sessions/foo/sess_abc.jsonl
  ← return messages
```

## aginx 侧兼容性

aginx `AcpStdioAdapter` 的 `list_sessions`：
- `storage_path = ~/.opencarrier/sessions`
- `storage_format` 未设置 → 走 `scan_claude_sessions`
- `scan_claude_sessions` 遍历 `storage_dir` 下一层子目录，找 `.jsonl`
- `parse_jsonl_metadata` 读取前 30 行解析 `title`/`lastMessage`/`workdir`

完全兼容，无需修改 aginx 代码。

## 实施步骤

1. 创建 `acp_session.rs`
2. 更新 `lib.rs` 导出，删除 `yinghe_session.rs`
3. 简化 `serve.rs`（移除 yingheclient + legacy JSON-RPC，接 `AcpSessionStore`）
4. 改造 `acp.rs`（全面持久化，`session/list`、`_aginx/*` 全部走文件）
5. 更新 `aginx.toml`
6. `cargo check` / `cargo test -p opencarrier-memory -p opencarrier-cli`
7. 运行验证：启动 `opencarrier serve` + aginx，测试 session/new、prompt、listConversations、getMessages

## 保留说明

- `MemorySubstrate` / `SessionStore`（SQLite `opencarrier.db`）**保留不变**，继续负责内核 `CanonicalSession`、KV、memories、task queue 等内部存储
- 旧 `yinghe_sessions.db` 中的会话**不迁移**，删除代码后自然失效
- 旧 yingheclient ChatRequest 格式输入**不再支持**，客户端需使用标准 ACP
