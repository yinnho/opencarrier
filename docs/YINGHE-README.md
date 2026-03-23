# yinghe - 应合载体

> Rust 版应合载体，与 yingheclient 功能对等

## 安装

### 从源码编译

```bash
git clone https://github.com/yinnho/opencarrier.git
cd opencarrier
cargo build --release -p opencarrier-cli
```

二进制文件位于 `target/release/yinghe`

### 一键安装

```bash
curl -fsSL https://carrier.yinnho.cn/install.sh | sh
```

## 命令

```bash
yinghe serve           # serve 模式 (stdin/stdout, 供 agentd 调用)
yinghe status          # 查看状态
yinghe bind <code>     # 绑定配对码
yinghe unbind          # 解绑
yinghe --help          # 显示帮助
```

## serve 模式

serve 模式从 stdin 读取请求，输出响应到 stdout。所有日志输出到 stderr。

### 支持的协议

1. **JSON-RPC 2.0**

```json
{"jsonrpc": "2.0", "id": 1, "method": "sendMessage", "params": {"message": "Hello"}}
```

2. **yingheclient ChatRequest**

```json
{"type": "chat", "conversationId": "conv-001", "conversationType": "carrier", "chatType": "direct", "content": "Hello"}
```

### 响应格式

```json
{"type": "chat_response", "conversationId": "conv-001", "response": "Hello!", "metadata": {"rounds": 1}}
```

## 配置

配置文件路径: `~/.opencarrier/config.toml`

```toml
[general]
provider = "proxy"        # 使用云端代理

[proxy]
base_url = "https://api.yinghe.plus"
```

## 对话类型

| 类型 | 说明 |
|------|------|
| `carrier` | 载体模式 - 完整权限 |
| `plugin` | Plugin 模式 - 受限工具 |
| `avatar` | Avatar 模式 - 支持群聊 |
| `role` | Role 模式 |

## 群聊支持

- `mentioned`: 被 @ 提及时响应
- `implicitMention`: 回复 Agent 消息时响应
- 无提及时不响应 (避免干扰群聊)

## 性能

| 指标 | 目标 | 实际 |
|------|------|------|
| 启动时间 | < 100ms | 2ms |
| 内存占用 | < 50MB | 17MB |
| 二进制大小 | < 50MB | 38MB |

## 与 TypeScript 版本兼容性

本实现与 yingheclient (TypeScript) 功能对等:

- ✅ ChatRequest/ChatResponse 协议
- ✅ 会话持久化 (SQLite)
- ✅ 群聊 @ 提及检测
- ✅ 多对话类型支持
- ✅ ProxyLLM 云端代理
- ✅ Relay WebSocket 连接
- ✅ 配对码绑定

## 测试

```bash
# 运行所有测试
cargo test --workspace

# 运行 yingheclient 协议测试
cargo test -p opencarrier-types --test yingheclient_protocol

# 运行 TypeScript 对比测试
cargo test -p opencarrier-types --test typescript_comparison_test

# 运行 serve 模式测试
cargo test -p opencarrier-cli --test serve_mode_test
```

## 架构

```
crates/
├── opencarrier-cli/          # CLI (yinghe 命令)
│   └── src/serve.rs          # serve 模式实现
├── opencarrier-types/
│   └── src/yinghe.rs         # yingheclient 类型定义
├── opencarrier-memory/
│   └── src/yinghe_session.rs # 会话管理
├── ying-relay/               # Relay WebSocket 连接
└── opencarrier-runtime/
    ├── drivers/proxy.rs      # ProxyLLM 驱动
    └── cloud_client.rs       # 云端客户端
```

## License

MIT
