# Claw Code 项目分析参考

> 来源: `/Users/sophiehe/Documents/yinnhoos/claw-code-main/`
> 分析日期: 2026-04-01

## 项目概况

Claw Code 是对 Claude Code (原 Anthropic CLI) 的开源重写项目，由韩国开发者 instructkr 创建。
Python + Rust 双实现，Rust 端为核心。

Rust workspace 结构 (`rust/crates/`):

| Crate | 职责 |
|-------|------|
| `api` | API 客户端，多 Provider 抽象，SSE 流解析 |
| `runtime` | 会话状态，上下文压缩，MCP 集成，prompt 构建 |
| `tools` | 工具定义，执行框架，子 Agent 限制 |
| `commands` | 斜杠命令注册/分发，技能发现 |
| `plugins` | 插件系统，Hook 管道 |
| `server` | HTTP/SSE 会话服务 (Axum) |
| `lsp` | LSP 客户端集成 |
| `claw-cli` | 交互式 REPL |
| `compat-harness` | 编辑器兼容层 |

---

## 一、Provider 抽象层 (最有借鉴价值)

### 架构层次

```
ProviderClient (enum 调度层)        ← client.rs
    ├── ClawApiClient               ← claw_provider.rs (Anthropic API)
    └── OpenAiCompatClient          ← openai_compat.rs (xAI/OpenAI)

Provider trait (接口层)             ← providers/mod.rs
    ├── send_message() → 非流式
    └── stream_message() → 流式，返回 Self::Stream

MessageStream (enum 统一流)        ← client.rs
    ├── ClawApi(MessageStream)      → SseParser 解析 Anthropic SSE
    └── OpenAiCompat(MessageStream) → OpenAiSseParser + StreamState 状态机

MODEL_REGISTRY (静态路由表)         ← providers/mod.rs
    模型名/别名 → ProviderMetadata (provider, auth_env, base_url_env)
```

### Provider trait 定义

```rust
// providers/mod.rs
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

pub trait Provider {
    type Stream;
    fn send_message<'a>(&'a self, request: &'a MessageRequest) -> ProviderFuture<'a, MessageResponse>;
    fn stream_message<'a>(&'a self, request: &'a MessageRequest) -> ProviderFuture<'a, Self::Stream>;
}
```

关键点:
- 使用 `ProviderFuture` 类型别名封装 `Pin<Box<dyn Future>>`，避免在每个实现中重复写
- `send_message` 和 `stream_message` 的生命周期绑定 `&self` 和 `&request`
- 两个 Provider (`ClawApiClient`, `OpenAiCompatClient`) 各自实现此 trait

### ProviderClient 统一调度

```rust
// client.rs
pub enum ProviderClient {
    ClawApi(ClawApiClient),
    Xai(OpenAiCompatClient),
    OpenAi(OpenAiCompatClient),
}

impl ProviderClient {
    pub fn from_model(model: &str) -> Result<Self, ApiError> {
        let resolved = resolve_model_alias(model);
        match detect_provider_kind(&resolved) {
            ProviderKind::ClawApi => Ok(Self::ClawApi(ClawApiClient::from_env()?)),
            ProviderKind::Xai => Ok(Self::Xai(OpenAiCompatClient::from_env(OpenAiCompatConfig::xai())?)),
            ProviderKind::OpenAi => Ok(Self::OpenAi(OpenAiCompatClient::from_env(OpenAiCompatConfig::openai())?)),
        }
    }
}
```

关键点:
- `from_model()` 是入口，根据模型名自动选择 Provider
- `MessageStream` 也是 enum 包装，`next_event()` 统一接口

### 模型注册表和别名

```rust
// providers/mod.rs
const MODEL_REGISTRY: &[(&str, ProviderMetadata)] = &[
    ("opus", ProviderMetadata { provider: ProviderKind::ClawApi, auth_env: "ANTHROPIC_API_KEY", ... }),
    ("grok", ProviderMetadata { provider: ProviderKind::Xai, auth_env: "XAI_API_KEY", ... }),
    // ...
];

pub fn resolve_model_alias(model: &str) -> String {
    // "opus" → "claude-opus-4-6", "grok" → "grok-3", etc.
}

pub fn detect_provider_kind(model: &str) -> ProviderKind {
    // 1. 先查 MODEL_REGISTRY
    // 2. 回退到检查环境变量 (ANTHROPIC_API_KEY, OPENAI_API_KEY, XAI_API_KEY)
}
```

### OpenAI 兼容层 — 请求格式转换

`openai_compat.rs:634` `build_chat_completion_request()` 核心转换逻辑:

| Anthropic 格式 | OpenAI 格式 |
|---------------|-------------|
| `system` 字段 | `role: "system"` 消息 |
| `ToolResult` block | `role: "tool"` + `tool_call_id` |
| `ToolDefinition(name, description, input_schema)` | `type: "function"` 包装 |
| `ToolChoice::Auto` | `"auto"` |
| `ToolChoice::Any` | `"required"` |
| `ToolChoice::Tool { name }` | `{type: "function", function: {name}}` |

### OpenAI 流式响应 — StreamState 状态机

`openai_compat.rs:300-469` 将 OpenAI chunk 实时规范化为 Anthropic 的 `StreamEvent`:

```
OpenAI chunk (ChatCompletionChunk)
    │
    ▼ StreamState.ingest_chunk()
    │
    ├── 首个 chunk → emit MessageStart
    ├── delta.content → emit ContentBlockStart(Text) + ContentBlockDelta(TextDelta)
    ├── delta.tool_calls → ToolCallState 累积 id/name/arguments
    │   ├── id+name 就绪 → emit ContentBlockStart(ToolUse)
    │   └── arguments 增量 → emit ContentBlockDelta(InputJsonDelta)
    ├── finish_reason="tool_calls" → emit ContentBlockStop
    └── usage → 记录到 self.usage

finish() 时关闭所有未关闭的 block，emit MessageDelta + MessageStop
```

`ToolCallState` 关键字段:
- `id: Option<String>` — 增量累积
- `name: Option<String>` — 增量累积
- `arguments: String` — 拼接增量 JSON
- `emitted_len: usize` — 跟踪已发射的 arguments 长度
- `block_index()` = `openai_index + 1`（因为 index 0 是 text block）

finish_reason 归一化: `"stop"` → `"end_turn"`, `"tool_calls"` → `"tool_use"`

### 认证源链

```rust
// claw_provider.rs
pub enum AuthSource {
    None,
    ApiKey(String),
    BearerToken(String),
    ApiKeyAndBearer { api_key: String, bearer_token: String },
}
```

优先级: `ANTHROPIC_API_KEY` > `ANTHROPIC_AUTH_TOKEN` > 磁盘 OAuth token (可自动刷新)

`AuthSource::apply()` 同时设置 `x-api-key` header 和 `Authorization: Bearer` header

### 统一重试策略

两个 Provider 共享:
- 指数退避: 200ms × 2, 最大 2s, 最多 2 次重试
- 可重试状态码: `408, 409, 429, 500, 502, 503, 504`
- `backoff_for_attempt()` 使用 `checked_shl` 防止溢出
- `RetriesExhausted` 包装最后一次错误

### SSE 解析器

`sse.rs` — Anthropic 原生 SSE:
- 字节缓冲区，累积 chunk
- 扫描 `\n\n` 或 `\r\n\r\n` 分隔符提取帧
- 过滤 `ping` 事件和 `[DONE]` 标记
- 支持多行 `data:` 拼接 JSON
- `finish()` 处理尾部不完整数据

`openai_compat.rs` — OpenAI SSE:
- 同样的帧提取逻辑 (`OpenAiSseParser`)
- 但反序列化为 `ChatCompletionChunk` 而非 `StreamEvent`
- 通过 `StreamState` 转换为 Anthropic 格式的 `StreamEvent`

### 类型系统

`types.rs` 核心类型:

```rust
// 请求
MessageRequest { model, max_tokens, messages, system?, tools?, tool_choice?, stream }
InputMessage { role, content: Vec<InputContentBlock> }
InputContentBlock: Text | ToolUse | ToolResult
ToolDefinition { name, description?, input_schema }
ToolChoice: Auto | Any | Tool { name }

// 响应
MessageResponse { id, kind, role, content: Vec<OutputContentBlock>, model, stop_reason?, usage }
OutputContentBlock: Text | ToolUse | Thinking | RedactedThinking
Usage { input_tokens, cache_creation_input_tokens, cache_read_input_tokens, output_tokens }

// 流事件
StreamEvent: MessageStart | MessageDelta | ContentBlockStart | ContentBlockDelta | ContentBlockStop | MessageStop
ContentBlockDelta: TextDelta | InputJsonDelta | ThinkingDelta | SignatureDelta
```

所有 enum 用 `#[serde(tag = "type", rename_all = "snake_case")]` 做标记序列化。

### 错误处理

```rust
// error.rs
pub enum ApiError {
    MissingCredentials { provider, env_vars },
    ExpiredOAuthToken,
    Auth(String),
    InvalidApiKeyEnv(VarError),
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Api { status, error_type?, message?, body, retryable },
    RetriesExhausted { attempts, last_error },
    InvalidSseFrame(&'static str),
    BackoffOverflow { attempt, base_delay },
}
```

---

## 二、Runtime 和会话管理

### 会话数据模型

```rust
// runtime/src/session.rs
pub struct Session {
    pub version: u32,
    pub messages: Vec<ConversationMessage>,
}
```

`ConversationMessage { role: MessageRole, blocks: Vec<ContentBlock> }`
- 追加式日志，支持 JSON 持久化
- 使用自定义 `JsonValue` 解析器 (runtime/src/json.rs)

### 对话循环

`runtime/src/conversation.rs:153-263` `run_turn()`:

1. 推入用户消息
2. 构建 `ApiRequest`
3. 调用 `api_client.stream(request)` 获取事件
4. `build_assistant_message(events)` 组装响应
5. 提取待处理工具调用
6. **无工具调用则结束**
7. 有工具调用: 检查权限 → PreToolUse hook → 执行 → PostToolUse hook → 推入结果
8. 回到步骤 2

返回 `TurnSummary { assistant_messages, tool_results, iterations, cumulative_usage }`

### 上下文压缩

`runtime/src/compact.rs`:
- `estimate_session_tokens()` 估算 token 数
- `compact_session()` 移除旧消息，生成结构化摘要
- 摘要包含: 范围、工具名、最近请求、待办项、关键文件、时间线
- 增量压缩，保留历史摘要

### 核心 Traits

```rust
trait ApiClient {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError>;
}

trait ToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError>;
}

trait PermissionPrompter {
    fn decide(&mut self, request: &PermissionRequest) -> PermissionPromptDecision;
}
```

泛型核心: `ConversationRuntime<C: ApiClient, T: ToolExecutor>`

### MCP 集成

- 工具名命名空间: `mcp__{server}__{tool}`
- 六种传输: Stdio / Sse / Http / WebSocket / Sdk / ManagedProxy
- `McpStdioProcess`: tokio Child + Content-Length framed JSON-RPC
- `McpServerManager`: 管理多服务器生命周期 (spawn → initialize → tools/list → tools/call)

### 配置分层

```
~/.claw/config.json          (用户 legacy)
~/.claw/settings.json        (用户 settings)
.claw/config.json            (项目 legacy)
.claw/settings.json          (项目 settings)
.claw/settings.local.json    (本地覆盖)
```

深层合并，后层覆盖前层。

---

## 三、工具系统

### 工具注册

`tools/src/lib.rs` — 16 个内置工具:

| 工具 | 权限要求 |
|------|---------|
| bash, write_file, edit_file | WorkspaceWrite |
| read_file, glob_search, grep_search, WebFetch, WebSearch | ReadOnly |
| TodoWrite, Skill, Agent, ToolSearch, NotebookEdit, Sleep, SendUserMessage, Config, StructuredOutput, REPL, PowerShell | 各异 |

每个工具 `ToolSpec { name, description, input_schema, required_permission }`

### 执行分发

`execute_tool()` 是 match 语句，反序列化 JSON 到对应 Input 结构体，调用 `run_*` 函数。

### 子 Agent 工具限制

```rust
fn allowed_tools_for_subagent(subagent_type) -> Vec<&str> {
    // Explore: 只读 (无 bash, 无写)
    // Plan: Explore + TodoWrite, SendUserMessage
    // Verification: 只读 + bash, PowerShell (无 write_file)
    // General-purpose: 除 Agent 外全部
}
```

### ToolSearch (模糊匹配)

- 评分: 精确名匹配 +8, 名包含 +4, canonical token +12, 描述匹配 +2/+3
- 语法: `select:ToolA,ToolB` 精确选择, `+term` 必选词

---

## 四、Plugin 系统

### 三种插件来源

| 类型 | 特点 |
|------|------|
| Builtin | 硬编码在 Rust 中，无 manifest |
| Bundled | 随应用发布在 `bundled/` 目录，有 `plugin.json` |
| External | 用户安装 (本地路径/Git URL)，完整生命周期管理 |

### Plugin trait

```rust
trait Plugin {
    fn metadata(&self) -> &PluginMetadata;
    fn hooks(&self) -> &PluginHooks;
    fn lifecycle(&self) -> &PluginLifecycle;
    fn tools(&self) -> &[PluginTool];
    fn validate(&self) -> Result<(), PluginError>;
    fn initialize(&self) -> Result<(), PluginError>;
    fn shutdown(&self) -> Result<(), PluginError>;
}
```

### Hook 系统

`plugins/src/hooks.rs` — `HookRunner`:

- 两个事件: `PreToolUse`, `PostToolUse`
- 执行: 子进程，JSON payload 通过 stdin，环境变量设置上下文
- 退出码: `0` = 允许, `2` = 拒绝 (阻止工具执行), 其他 = 警告 (允许但记录)
- 跨平台: Windows `cmd /C`, Unix `sh <file>` 或 `sh -lc <expression>`

### Plugin Manager

- `install(source)` — 本地路径或 Git URL
- `enable(id)` / `disable(id)` — 持久化到 settings.json
- `uninstall(id)` — 删除文件和注册条目 (bundled 只能禁用)
- `update(id)` — 从源重新同步
- `plugin_registry()` — 发现所有插件，聚合 hooks 和 tools

---

## 五、Slash 命令系统

### 注册

28 个命令的静态注册表:
```rust
struct SlashCommandSpec {
    name: &'static str,
    aliases: &[&static str],
    summary: &'static str,
    argument_hint: Option<&'static str>,
    resume_supported: bool,
}
```

### 双层分发

- `handle_slash_command()` — 处理需要 Session 的命令 (Help, Compact)
- 其他命令返回 `None`，由独立 handler 函数处理

### Agent/Skill 发现

- Agent: `.codex/agents/*.toml` 和 `.claw/agents/*.toml`
- Skill: `skills/*/SKILL.md` (YAML frontmatter) 和 `commands/*.md`
- 搜索路径: 项目目录向上 → `$CODEX_HOME` → `~/.codex` → `~/.claw`
- 支持 shadowing: 项目级同名覆盖用户级

---

## 六、HTTP Server

`server/src/lib.rs` — Axum 单文件实现:

| 端点 | 方法 | 功能 |
|------|------|------|
| `/sessions` | POST | 创建会话 |
| `/sessions` | GET | 列出会话 |
| `/sessions/{id}` | GET | 获取会话详情 |
| `/sessions/{id}/events` | GET | SSE 事件流 |
| `/sessions/{id}/message` | POST | 发送消息 |

SSE 端点使用 `broadcast::Sender<SessionEvent>` (容量 64) + `KeepAlive` (15s)

---

## 七、对 OpenCarrier 的借鉴建议

### 高优先级

1. **Provider 抽象层** — 整体移植设计模式
   - 定义 `Provider` trait (`send_message + stream_message`)
   - 实现 Groq 提供商 (兼容 OpenAI 格式，复用 `OpenAiCompatClient` 模式)
   - 加 `ProviderClient` enum 按 model 名自动路由
   - 复用 `StreamState` 状态机规范化流式响应
   - 加 `MODEL_REGISTRY` 支持 grok、llama、deepseek 等别名

2. **Plugin Hook 系统** — 子进程模式比 WASM 更灵活
   - `PreToolUse / PostToolUse` 事件
   - 退出码控制: 0=允许, 2=拒绝
   - JSON payload 通过 stdin + 环境变量

3. **子 Agent 工具限制** — 按角色限制可用工具
   - Explore: 只读
   - Plan: 只读 + TodoWrite
   - Verification: 只读 + bash
   - General-purpose: 除 Agent 外全部

### 中优先级

4. **上下文压缩** — 结构化摘要模式
5. **模型别名系统** — 短名到全名的映射
6. **统一重试策略** — 指数退避 + 可重试状态码

### 低优先级

7. **Skill 发现系统** — 多路径搜索 + shadowing
8. **Slash 命令双层分发**
9. **SSE Server 模式** — broadcast channel + KeepAlive

---

## 八、CLI (claw-cli)

### 文件结构

| 文件 | 职责 |
|------|------|
| `main.rs` | 入口、参数解析、REPL 循环、运行时组装、流式渲染 |
| `input.rs` | 自定义行编辑器 (Vim 模式、多行编辑、Tab 补全) |
| `render.rs` | 终端 Markdown 渲染器、Spinner、语法高亮 |
| `init.rs` | 项目初始化 (CLAW.md, .claw/, .gitignore) |
| `args.rs` | Clap 参数定义 (旧版，未被 main.rs 使用) |
| `app.rs` | 早期 CLI 抽象 (保留作备选) |

### 参数解析 (手写，非 Clap)

`main.rs` 的 `parse_args()` 手动遍历 `env::args()`，返回 `CliAction` enum:

```rust
enum CliAction {
    DumpManifests, BootstrapPlan,
    Agents { args }, Skills { args },
    PrintSystemPrompt { cwd, date },
    Version, ResumeSession { session_path, commands },
    Prompt { prompt, model, output_format, allowed_tools, permission_mode },
    Login, Logout, Init,
    Repl { model, allowed_tools, permission_mode },
    Help,
}
```

关键行为:
- 无参数 → 进入 REPL
- 裸文本 (如 `claw explain this`) → 一次性 Prompt
- `-p "text"` → 兼容 Claw Code 的一次性 prompt
- `--resume session.json /cmd` → 对保存的会话执行命令
- `/agents`, `/skills` 可从 shell 直接调用

### REPL 循环

```
run_repl(model, allowed_tools, permission_mode)
    │
    ├── 构建 LiveCli (组装 runtime + session + system_prompt)
    ├── 创建 LineEditor (Vim 模式行编辑器)
    │
    └── loop {
        match editor.read_line() {
            Submit(input) →
                /exit, /quit → 持久化 + break
                斜杠命令 → handle_repl_command
                其他 → push_history + run_turn
            Cancel → 清空输入
            Exit → 持久化 + break
        }
    }
```

### 运行时组装 (关键接线)

`build_runtime()` 是核心接线函数:

```rust
fn build_runtime(session, model, system_prompt, enable_tools, emit_output,
                 allowed_tools, permission_mode, progress_reporter)
    -> ConversationRuntime<DefaultRuntimeClient, CliToolExecutor>
{
    let (feature_config, tool_registry) = build_runtime_plugin_state()?;
    ConversationRuntime::new_with_features(
        session,
        DefaultRuntimeClient::new(model, ...),      // API 客户端
        CliToolExecutor::new(allowed_tools, ...),    // 工具执行器
        permission_policy(permission_mode, &tool_registry),
        system_prompt,
        feature_config,                              // 插件特性
    )
}
```

- `DefaultRuntimeClient` — 包装 `ClawApiClient`，处理流式 SSE、工具输入累积
- `CliToolExecutor` — 检查 allowlist → 委托 `GlobalToolRegistry::execute()`
- `build_runtime_plugin_state()` — 加载插件，聚合 hooks 和 tools

### 流式工具输入累积

关键细节: API 发送工具调用时，`ContentBlockStart` 携带空 JSON `{}`，真正的输入通过后续 `InputJsonDelta` 增量到达。
代码在 `ContentBlockStop` 时才完成累积，避免处理不完整的输入。

### 内部 Prompt 系统

`/commit`, `/pr`, `/issue`, `/bughunter`, `/ultraplan` 等命令通过创建新的 `ConversationRuntime` 发送精心构造的 prompt，将 LLM 作为内部服务使用。

### 模型切换 = 运行时重建

`/model` 切换模型时会重建整个 `ConversationRuntime`，保留现有 session。`/permissions` 和 `/compact` 同理。

### 工具输出截断

显示截断 (80行/6000字符 for reads, 60行/4000字符 for others)，但完整结果保存在 session JSON 中。

### 自定义行编辑器

`input.rs` 基于 crossterm raw mode 的完整 Vim 模拟:
- Normal/Insert/Visual/Command 四种模式
- `h/j/k/l`, `dd`(删行), `yy`(复制行), `p`(粘贴)
- Visual 模式用 ANSI reverse video 渲染选中区域
- Shift+Enter/Ctrl+J 插入换行，Enter 提交
- Tab 补全斜杠命令
- 非终端环境回退到简单 `read_line`

### Markdown 渲染

`render.rs` 基于 `pulldown-cmark` + `syntect`:
- 完整 CommonMark 解析: 标题、加粗、代码块、表格、引用、列表
- 代码块: `╭─ language` / `╰─` 边框，syntect 语法高亮 (base16-ocean.dark)
- 流式渲染: `MarkdownStreamState` 在安全边界 (空行或关闭的代码围栏) 才渲染，避免渲染不完整 Markdown
- Spinner: Braille 动画 (⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏)，蓝/绿/红色
- 表格: Unicode box-drawing 字符 (│, ┼, ─)

### 每工具格式化显示

- `bash`: 暗背景命令框
- `read_file`: 文件路径 + 行范围
- `write_file`: 文件路径 + 行数
- `edit_file`: 红绿 diff 预览
- `glob/grep`: 模式和范围

---

## 九、HTTP Server (server crate)

单文件实现 (`src/lib.rs`, 442 行)，Axum 框架。

### 路由

| 方法 | 路径 | 功能 |
|------|------|------|
| POST | `/sessions` | 创建会话 |
| GET | `/sessions` | 列出会话 |
| GET | `/sessions/{id}` | 获取完整会话 |
| GET | `/sessions/{id}/events` | SSE 事件流 |
| POST | `/sessions/{id}/message` | 发送消息 |

### SSE 实现

- 每个 Session 持有 `broadcast::Sender<SessionEvent>` (容量 64)
- 新连接先收到 `Snapshot` (完整状态同步)，再接收增量 `Message` 事件
- `Lagged` 错误静默跳过 (不中断连接)
- `Closed` 中断循环
- `KeepAlive` 每 15s 发送心跳

### 状态管理

```rust
AppState { sessions: Arc<RwLock<HashMap<SessionId, Session>>>, next_session_id: Arc<AtomicU64> }
Session { id, created_at: u64 (ms), conversation: RuntimeSession, events: broadcast::Sender }
```

- `RwLock` 允许多个并发读者
- `send_message` 的写锁最小化: 克隆 sender 后立即释放锁再广播
- 无认证、无 CORS、无中间件
- 会话纯内存，无持久化，无 TTL

### 与 Runtime 的关系

非常薄: 只导入 `ConversationMessage` 和 `Session`。
Server 只做消息存储和 SSE 广播，不调用 LLM。
`send_message` 只追加用户消息并广播，不触发 assistant 响应。

---

## 十、compat-harness

小工具 crate (357 行)，解析上游 TypeScript 源码提取结构化清单。

功能:
- 定位上游 TS 源文件 (多路径搜索)
- 提取 `CommandRegistry` (Builtin / InternalOnly / FeatureGated)
- 提取 `ToolRegistry` (Base / Conditional)
- 提取 `BootstrapPlan` (启动阶段序列)
- 通过 `dump-manifests` 和 `bootstrap-plan` 子命令暴露

用途: 开发时内省，对比 Rust 实现与原版的差距。
