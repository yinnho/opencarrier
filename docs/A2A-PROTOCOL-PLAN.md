# A2A 协议应用及开发计划

> **版本**: v1.0
> **日期**: 2026-03-20
> **状态**: 规划中

---

## 1. 概述

### 1.1 什么是 A2A

A2A（Agent-to-Agent）是一种让不同 Agent 之间能够"说同一种语言"的通信协议。

**场景举例**：旅行 Agent 需要协调航班 Agent（查机票）、酒店 Agent（订住宿）、翻译 Agent（实时翻译）——没有 A2A 时，这些 Agent 各说各话；有了 A2A，它们能通过统一协议协商任务、共享状态。

### 1.2 技术基础

- **传输层**: JSON-RPC 2.0 over TCP
- **消息格式**: JSON（每条消息以 `\n` 结尾）
- **默认端口**: TCP 86
- **核心机制**: Agent Card + 任务生命周期管理

---

## 2. 架构设计

### 2.1 整体架构

**agentd 是反向代理**（类似 nginx），监听 TCP 86 端口，转发请求到后端 Agent 进程：

```
┌─────────────────────────────────────────────────────────────────┐
│                      Agent 服务器（公网 IP）                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│                      App ──A2A──→ agentd (反向代理)              │
│                              (监听 TCP 86)                       │
│                                    │                            │
│              ┌─────────────────────┼─────────────────────┐      │
│              │                     │                     │      │
│              ▼                     ▼                     ▼      │
│         opencarrier          yingheclient            claude     │
│          (upstream)           (upstream)            (upstream)  │
│         stdin/stdout         stdin/stdout          stdin/stdout │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 2.2 agentd 的角色（类似 nginx）

agentd 是**反向代理/网关**，职责类似 nginx：

| 功能 | 说明 |
|------|------|
| 监听端口 | TCP 86 |
| 请求转发 | 将 A2A 请求转发给后端 Agent |
| 进程管理 | spawn 并管理 Agent 进程 |
| 透传消息 | 不处理业务逻辑，只做转发 |

### 2.3 opencarrier 的角色

opencarrier 是**后端服务**（upstream）：

- 由 agentd spawn 启动
- 从 stdin 读取 A2A 请求
- 处理业务逻辑（调用 LLM、执行工具等）
- 输出 A2A 响应到 stdout
- 日志输出到 stderr

### 2.4 消息流

```
App 发送消息（A2A 格式）
    ↓
agentd 接收（反向代理，监听 86 端口）
    ↓
agentd 通过 stdin 转发给 opencarrier（A2A 格式透传）
    ↓
opencarrier 处理，输出到 stdout（A2A 格式）
    ↓
agentd 返回给 App（A2A 格式透传）
```

**关键点**：
- agentd 像 nginx 一样透传消息，不解析 A2A 协议内容
- opencarrier 负责解析和处理 A2A 协议
- 整个链路消息格式统一为 **A2A 协议**

---

## 3. 协议规范（JSON-RPC 2.0）

### 3.1 请求格式

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "sendMessage",
  "params": {
    "agentId": "carrier",
    "message": "你好"
  }
}
```

### 3.2 响应格式（成功）

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "response": "你好！有什么可以帮你的？"
  }
}
```

### 3.3 响应格式（错误）

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32601,
    "message": "Method not found"
  }
}
```

### 3.4 JSON-RPC 错误码

| 代码 | 含义 |
|------|------|
| -32700 | Parse error - 无效 JSON |
| -32600 | Invalid Request - 无效请求 |
| -32601 | Method not found - 方法不存在 |
| -32602 | Invalid params - 无效参数 |
| -32603 | Internal error - 内部错误 |

### 3.5 记忆压缩方法

**请求（App → Carrier）**：

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "compactMemory",
  "params": {
    "messages": [
      {"role": "user", "content": "..."},
      {"role": "assistant", "content": "..."}
    ],
    "keepRecent": 50
  }
}
```

**响应（Carrier → App）**：

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "summary": "对话摘要：用户询问了 X，Agent 建议了 Y...",
    "recentMessages": [
      {"role": "user", "content": "..."},
      {"role": "assistant", "content": "..."}
    ],
    "compactedCount": 50
  }
}
```

### 3.5 主要方法

| 方法 | 说明 |
|------|------|
| `hello` | 客户端握手 |
| `sendMessage` | 发送消息给 Agent |
| `compactMemory` | 记忆压缩（App 发起，Carrier 执行）|
| `getAgentCard` | 获取 Agent 能力卡片 |
| `listAgents` | 列出所有 Agent |
| `bye` | 关闭连接（通知，无响应）|

> **架构原则**: App 管理记忆，Carrier 执行压缩。详见 [ARCHITECTURE-PRINCIPLES.md](./ARCHITECTURE-PRINCIPLES.md)

### 3.6 流式事件

流式响应使用事件格式（非 JSON-RPC）：

```json
{"type":"taskStatus","taskId":"task-123","status":"running"}
{"type":"artifact","taskId":"task-123","artifact":{"content":"..."}}
{"type":"taskCompleted","taskId":"task-123","result":{"status":"ok"}}
```

---

## 4. Agent Card（智能体名片）

### 4.1 概述

每个 A2A 智能体需提供 Agent Card（JSON 格式），通过 `/.well-known/agent.json` 访问。

### 4.2 包含内容

| 字段 | 说明 | 示例 |
|------|------|------|
| 身份信息 | 名称、版本、描述 | `{"name": "opencarrier", "version": "0.1.0"}` |
| 能力列表 | 支持的任务类型、模态 | `"capabilities": ["text", "file"]` |
| 通信端点 | URL、支持的传输方式 | `"endpoints": [{"url": "...", "transport": "sse"}]` |
| 认证要求 | OAuth 2.0、API Key 等 | `"authentication": {"schemes": ["bearer"]}` |

---

## 5. 任务生命周期

### 5.1 状态流转

A2A 将任务状态标准化为 5 种：

```
submitted → working → input-required → completed
                                      ↘ failed
```

### 5.2 状态说明

| 状态 | 说明 |
|------|------|
| `submitted` | 任务已提交，等待处理 |
| `working` | 正在处理中 |
| `input-required` | 需要额外输入（等待用户） |
| `completed` | 任务完成 |
| `failed` | 任务失败 |

---

## 6. opencarrier 的实现

### 6.1 当前问题

opencarrier 当前实现**错误**地直接连接 Relay WebSocket：

```
❌ 错误：opencarrier ──WebSocket──→ Relay
```

这不符合 agentd 托管架构。

### 6.2 正确实现

opencarrier 应该是 agentd 的**后端服务**（upstream），通过 stdin/stdout 通信：

```
✅ 正确：agentd ──stdin──→ opencarrier ──stdout──→ agentd
```

### 6.3 serve 模式

opencarrier 需要实现 `serve` 子命令，由 agentd 调用：

```bash
# agentd spawn opencarrier
opencarrier serve
```

**输入（stdin，JSON-RPC 格式）**：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "sendMessage",
  "params": {
    "agentId": "carrier",
    "message": "你好"
  }
}
```

**输出（stdout，JSON-RPC 格式）**：

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "response": "你好！有什么可以帮你的？"
  }
}
```

### 6.4 流式响应

支持流式输出：

```json
{"type":"taskStatus","taskId":"task-1","status":"running"}
{"type":"artifact","taskId":"task-1","artifact":{"content":"你好"}}
{"type":"artifact","taskId":"task-1","artifact":{"content":"！有什么"}}
{"type":"taskCompleted","taskId":"task-1","result":{"response":"..."}}
```

### 6.5 日志输出

所有日志**必须**输出到 stderr，不能污染 stdout：

```rust
// 正确：日志到 stderr
tracing::info!("Processing message");

// 错误：会污染 stdout
println!("Processing message");
```

---

## 7. 与 yingheclient 的关系

### 7.1 yingheclient 的实现

yingheclient 已经实现了 `serve` 模式：

```bash
ying serve  # 从 stdin 读取，输出到 stdout
```

### 7.2 opencarrier 的目标

opencarrier 需要实现与 yingheclient `serve` 模式**完全相同**的 stdin/stdout 接口，以便 agentd 可以无缝切换使用两者。

---

## 8. 开发计划

### Phase 1: JSON-RPC 协议实现 ✅

- [x] 定义 JSON-RPC 消息类型（Request/Response/Notification）
- [x] 实现 JSON-RPC 2.0 解析/序列化
- [ ] 复用 agent-cli/shared/src/protocol/jsonrpc.rs（可选优化）

### Phase 2: serve 模式实现 ✅

- [x] 添加 `opencarrier serve` 子命令
- [x] 从 stdin 读取 JSON-RPC 请求
- [x] 处理 `sendMessage` 方法
- [x] 输出 JSON-RPC 响应到 stdout
- [x] 所有日志输出到 stderr

### Phase 3: 流式响应

- [ ] 实现流式事件输出
- [ ] 支持 `taskStatus`、`artifact`、`taskCompleted` 事件

### Phase 4: Agent Card

- [x] 实现 Agent Card 结构（复用 opencarrier-runtime::a2a）
- [x] 声明能力和端点

### Phase 5: 与 agentd 集成测试

- [ ] 测试 agentd → opencarrier 通信
- [ ] 测试流式响应
- [ ] 测试错误处理

---

## 9. 代码结构

### 9.1 复用 agent-cli

opencarrier 可以直接复用 agent-cli 的协议实现：

```
agent-cli/shared/src/protocol/
├── jsonrpc.rs      # JSON-RPC 2.0 实现 ← 复用
├── message.rs      # 消息类型
├── agent_card.rs   # Agent Card
└── parser.rs       # 消息解析
```

### 9.2 新增文件

```
crates/opencarrier-cli/src/
└── commands/
    └── serve.rs    # serve 子命令实现
```

### 9.3 修改文件

```
crates/opencarrier-cli/src/
└── main.rs        # 添加 serve 子命令
```

---

## 10. App 端实现方案

### 10.1 方案选择

**App 自己用 Kotlin 实现 JSON-RPC + TCP**（方案 C）

| 方案 | 说明 | 选择 |
|------|------|------|
| FFI 调用 | 编译 shared 为 .so，JNI 调用 | ❌ 复杂 |
| 子进程 apc | App spawn apc 进程 | ❌ 需打包二进制 |
| **Kotlin 实现** | App 自己实现 JSON-RPC + TCP | ✅ **采用** |
| HTTP 网关 | agentd 加 HTTP 支持 | ❌ 需改 agentd |

### 10.2 为什么选 Kotlin 实现

1. **协议简单**：JSON-RPC 2.0 就是 JSON 格式，TCP 连接
2. **无依赖**：不需要打包 Rust 库
3. **跨平台**：Kotlin 在 Android/iOS 都能用
4. **易维护**：协议改动直接改 Kotlin 代码

### 10.3 App 端代码示例

```kotlin
class AgentClient(private val host: String, private val port: Int = 86) {

    fun sendMessage(agentId: String, message: String): String {
        val socket = Socket(host, port)
        val writer = PrintWriter(socket.getOutputStream(), true)
        val reader = BufferedReader(InputStreamReader(socket.getInputStream()))

        try {
            // 1. Hello 握手
            writer.println(jsonRpc(1, "hello", mapOf(
                "clientName" to "yingheapp",
                "clientVersion" to "1.0"
            )))
            reader.readLine() // helloOk

            // 2. 发送消息
            writer.println(jsonRpc(2, "sendMessage", mapOf(
                "agentId" to agentId,
                "message" to message
            )))
            val response = reader.readLine()

            // 3. Bye 关闭
            writer.println(jsonRpcNotification("bye"))

            return parseResult(response)
        } finally {
            socket.close()
        }
    }

    private fun jsonRpc(id: Int, method: String, params: Any): String {
        return JSONObject().apply {
            put("jsonrpc", "2.0")
            put("id", id)
            put("method", method)
            put("params", JSONObject(params as Map<*, *>))
        }.toString()
    }

    private fun jsonRpcNotification(method: String): String {
        return JSONObject().apply {
            put("jsonrpc", "2.0")
            put("method", method)
        }.toString()
    }

    private fun parseResult(response: String): String {
        val json = JSONObject(response)
        return json.getJSONObject("result").getString("response")
    }
}
```

---

## 11. 关键设计决策

### 11.1 agentd 的定位

**agentd 是反向代理，类似 nginx**：

- 监听 TCP 86 端口
- 接收 App 请求，转发给后端 Agent 进程
- 透传消息，不解析协议内容
- 管理 Agent 进程生命周期（spawn、监控、重启）

### 10.2 opencarrier 的定位

**opencarrier 是后端服务（upstream）**：

- 由 agentd spawn 启动
- 从 stdin 读取请求，输出响应到 stdout
- 负责业务逻辑：调用 LLM、执行工具
- 不需要直接连接 Relay，由 agentd 统一对外服务

### 10.3 为什么使用 JSON-RPC 2.0

- 标准化协议，广泛支持
- 支持请求/响应和通知两种模式
- 明确的错误码定义
- 易于调试（JSON 格式可读）

---

## 11. apc - Agent Protocol Client

### 11.1 什么是 apc

`apc` 是 Agent Protocol 的命令行客户端，**类似 curl 之于 HTTP**。

```bash
# 发送消息给 Agent
apc agent://hotel.example.com "查询房间"

# 查询 Agent 能力
apc --capa agent://localhost/carrier

# 列出所有 Agent
apc --url agent://localhost:86 list

# 搜索 Agent
apc --find "type=hotel AND location=Beijing"
```

### 11.2 apc 的工作流程

```
apc ──JSON-RPC──→ agentd (TCP 86) ──stdin──→ opencarrier
                                             ↓
apc ←──JSON-RPC── agentd ←──stdout───────────┘
```

1. apc 连接到 agentd（TCP 86）
2. 发送 `hello` 握手
3. 发送 `sendMessage` 请求
4. 接收响应
5. 发送 `bye` 关闭连接

### 11.3 代码位置

```
agent-cli/
├── apc/                      # 客户端 CLI
│   └── src/
│       ├── main.rs           # 入口
│       ├── client/
│       │   └── tcp.rs        # TCP 客户端实现
│       └── output/           # 输出格式化
│
├── agentd/                   # 反向代理（类似 nginx）
│   └── src/
│       ├── agent/builtin/
│       │   └── process.rs    # 进程 Agent（stdin/stdout）
│       └── server/           # TCP 服务器
│
└── shared/                   # 共享库
    └── src/protocol/
        ├── jsonrpc.rs        # JSON-RPC 2.0 实现
        ├── message.rs        # 消息类型
        └── parser.rs         # 消息解析
```

---

## 12. opencarrier 需要做什么

### 12.1 serve 模式

opencarrier 需要实现 `serve` 子命令，由 agentd spawn：

```bash
# agentd 配置
[[agents]]
name = "carrier"
command = "opencarrier"
args = ["serve"]
```

### 12.2 消息处理流程

```rust
// 伪代码
fn serve() {
    loop {
        // 1. 从 stdin 读取一行 JSON-RPC 请求
        let line = stdin.read_line()?;

        // 2. 解析 JSON-RPC
        let request = JsonRpcRequest::from_json(&line)?;

        // 3. 根据 method 处理
        let response = match request.method.as_str() {
            "sendMessage" => handle_send_message(request.params)?,
            "getAgentCard" => handle_get_agent_card()?,
            _ => JsonRpcResponse::error(request.id, method_not_found()),
        };

        // 4. 输出响应到 stdout（带换行）
        stdout.write(response.to_json().as_bytes())?;
        stdout.write(b"\n")?;
        stdout.flush()?;
    }
}
```

### 12.3 关键要求

| 要求 | 说明 |
|------|------|
| stdin 读取 | 每行一个 JSON-RPC 请求 |
| stdout 输出 | 每行一个 JSON-RPC 响应 |
| stderr 日志 | 所有日志输出到 stderr，**不能污染 stdout** |
| 消息分隔 | 每条消息以 `\n` 结尾 |

---

## 13. 参考资料

### 13.1 相关代码

- `agent-cli/shared/src/protocol/jsonrpc.rs` - JSON-RPC 2.0 实现（可直接复用）
- `agent-cli/shared/src/protocol/message.rs` - 消息类型定义
- `agent-cli/agentd/src/agent/builtin/process.rs` - agentd 进程管理
- `agent-cli/apc/src/client/tcp.rs` - apc 客户端实现
- `yingheclient/src/cli/serve.ts` - yingheclient serve 模式

### 13.2 协议规范

- `agent-cli/docs/PROTOCOL.md` - Agent Protocol v0.2 规范
- [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)

---

**最后更新**: 2026-03-20
**维护者**: 应合网络团队
