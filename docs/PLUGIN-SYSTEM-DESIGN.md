# OpenCarrier 插件系统 — 设计文档

## 1. 架构总览

```
┌─────────────────────────────────────────────────────┐
│                   OpenCarrier 主程序                  │
│                                                     │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────┐ │
│  │ Plugin      │  │ Bridge       │  │ Tool       │ │
│  │ Loader      │  │ Manager      │  │ Dispatcher │ │
│  └──────┬──────┘  └──────┬───────┘  └─────┬──────┘ │
│         │                │                 │        │
│         │ dlopen         │ ChannelMessage  │ ToolCall│
│         ▼                ▼                 ▼        │
│  ┌─────────────────────────────────────────────────┐│
│  │              Plugin ABI (C 接口层)               ││
│  │  register_channel() / register_tools()          ││
│  │  channel_start() / channel_send()               ││
│  │  tool_execute() / plugin_stop()                 ││
│  └─────────────────────────────────────────────────┘│
└─────────────────────────────┬───────────────────────┘
                              │
          ┌───────────────────┼───────────────────┐
          ▼                   ▼                   ▼
  ┌──────────────┐   ┌──────────────┐   ┌──────────────┐
  │ wecom plugin │   │telegram plugin│   │ notion plugin│
  │              │   │              │   │              │
  │ Channel:     │   │ Channel:     │   │ (tools only) │
  │  wecom_ws    │   │  telegram    │   │              │
  │  wecom_app   │   │              │   │ Tools:       │
  │  wecom_kf    │   │ Tools:       │   │  create_page │
  │              │   │  send_sticker│   │  query_db    │
  │ Tools:       │   │  manage_group│   │              │
  │  wedoc_xxx   │   │              │   │ Tenants:     │
  │  send_msg    │   │ Tenants:     │   │  workspace_1 │
  │              │   │  bot_token_A │   │  workspace_2 │
  │ Tenants:     │   │  bot_token_B │   │              │
  │  corp_A      │   │              │   │              │
  │  corp_B      │   │              │   │              │
  │  corp_C      │   │              │   │              │
  └──────────────┘   └──────────────┘   └──────────────┘
```

## 2. Plugin ABI — C 接口层

插件和主程序通过 C ABI 通信，避免 Rust ABI 不稳定的问题。

### 2.1 核心类型

```c
// === 不透明句柄 ===
typedef struct OCPlugin* OCPluginHandle;
typedef struct OCChannel* OCChannelHandle;
typedef struct OCTool* OCToolHandle;

// === 消息类型 ===
typedef enum {
    OC_CONTENT_TEXT,
    OC_CONTENT_IMAGE,
    OC_CONTENT_FILE,
    OC_CONTENT_VOICE,
    OC_CONTENT_LOCATION,
    OC_CONTENT_COMMAND,
} OCContentType;

typedef struct {
    OCContentType type;
    const char* text;           // TEXT: 文本内容
    const char* image_url;      // IMAGE: 图片 URL
    const char* image_caption;  // IMAGE: 可选标题
    const char* file_url;       // FILE: 文件 URL
    const char* file_name;      // FILE: 文件名
    const char* voice_url;      // VOICE: 语音 URL
    uint32_t voice_duration;    // VOICE: 时长(秒)
    double location_lat;        // LOCATION: 纬度
    double location_lon;        // LOCATION: 经度
    const char* command_name;   // COMMAND: 命令名
    const char* command_args;   // COMMAND: 命令参数 (JSON array)
} OCContent;

typedef struct {
    const char* channel_type;       // "wecom", "telegram", etc.
    const char* platform_message_id;
    const char* sender_id;          // 平台用户 ID
    const char* sender_name;        // 显示名
    const char* tenant_id;          // 租户标识 (corp_id / bot_id)
    OCContent content;
    uint64_t timestamp_ms;
    int is_group;
    const char* thread_id;          // 可选: 线程 ID
    const char* metadata_json;      // 可选: 平台特有元数据 (JSON)
} OCMessage;

typedef struct {
    const char* name;
    const char* description;
    const char* parameters_json;    // JSON Schema
} OCToolDef;

// === 回调函数类型 ===

// 主程序调用：发送消息给用户
typedef void (*OCSendCallback)(
    void* user_data,
    const char* tenant_id,
    const char* sender_id,
    OCContent content
);

// 主程序调用：发送消息到 channel（inbound）
typedef void (*OCMessageCallback)(
    void* user_data,
    OCMessage* message
);

// 主程序调用：执行 tool
typedef void (*OCToolExecuteCallback)(
    void* user_data,
    const char* tool_name,
    const char* args_json,      // 工具参数 (JSON)
    const char* context_json    // 调用上下文: {tenant_id, sender_id, agent_id}
);
```

### 2.2 插件导出函数

每个插件必须导出以下 C 函数：

```c
/// 插件名称（如 "wecom"）
extern const char* oc_plugin_name(void);

/// 插件版本（如 "1.0.0"）
extern const char* oc_plugin_version(void);

/// 最小 opencarrier 版本要求
extern const char* oc_plugin_min_version(void);

/// ABI 版本（主程序校验兼容性）
extern uint32_t oc_plugin_abi_version(void);

/// 初始化插件，传入配置 JSON + 主程序回调
/// config_json: plugin.toml 中的配置
/// send_fn: 主程序提供的消息发送回调（channel → agent 方向）
/// returns: 插件句柄，失败返回 NULL
extern OCPluginHandle oc_plugin_init(
    const char* config_json,
    OCMessageCallback message_cb,
    void* message_cb_user_data
);

/// 获取 channel 列表（一个插件可以有多个 channel）
/// out_channels: 输出 channel 句柄数组
/// returns: channel 数量
extern uint32_t oc_plugin_channels(
    OCPluginHandle plugin,
    OCChannelHandle** out_channels
);

/// 获取 channel 信息
extern const char* oc_channel_type(OCChannelHandle ch);
extern const char* oc_channel_name(OCChannelHandle ch);

/// 启动 channel（开始接收消息）
/// 消息通过 oc_plugin_init 传入的 message_cb 回调到主程序
extern int oc_channel_start(OCChannelHandle ch);

/// 主程序调用：通过 channel 发送消息给用户
extern int oc_channel_send(
    OCChannelHandle ch,
    const char* tenant_id,
    const char* user_id,
    OCContent content
);

/// 获取 tool 列表
/// out_tools: 输出 tool 定义数组
/// returns: tool 数量
extern uint32_t oc_plugin_tools(
    OCPluginHandle plugin,
    OCToolDef** out_tools
);

/// 执行 tool
/// args_json: 工具参数
/// context_json: {tenant_id, sender_id, agent_id, ...}
/// result_buf: 输出缓冲区
/// result_buf_len: 缓冲区大小
/// returns: 实际写入长度，< 0 表示错误
extern int oc_plugin_tool_execute(
    OCPluginHandle plugin,
    const char* tool_name,
    const char* args_json,
    const char* context_json,
    char* result_buf,
    uint32_t result_buf_len
);

/// 停止插件，释放资源
extern void oc_plugin_stop(OCPluginHandle plugin);
```

## 3. 主程序侧组件

### 3.1 PluginLoader

```
crates/opencarrier-runtime/src/plugin/
  mod.rs          — 模块入口
  loader.rs       — 扫描 plugins 目录，dlopen 加载
  abi.rs          — C ABI 类型和函数指针定义
  manager.rs      — 管理已加载插件的生命周期
```

**加载流程：**

```rust
impl PluginLoader {
    /// 扫描 ~/.opencarrier/plugins/，加载所有插件
    pub async fn load_all(&self) -> Result<Vec<LoadedPlugin>> {
        for plugin_dir in fs::read_dir(plugins_dir)? {
            let plugin_toml = plugin_dir.join("plugin.toml");
            let config = parse_plugin_config(&plugin_toml)?;

            // ABI 版本校验
            if config.abi_version != CURRENT_ABI_VERSION {
                warn!("Plugin {} ABI version mismatch", config.name);
                continue;
            }

            // 找到共享库
            let lib_path = find_shared_lib(&plugin_dir)?;

            // dlopen
            let lib = unsafe { Library::new(&lib_path)? };

            // 加载导出函数
            let plugin = load_plugin_symbols(&lib, &config)?;

            // 初始化
            let handle = (plugin.init)(
                config.to_json().as_ptr(),
                message_callback,
                callback_user_data,
            );

            // 注册 channel + tools
            let channels = (plugin.channels)(handle);
            let tools = (plugin.tools)(handle);

            // 注册到 BridgeManager 和 ToolDispatcher
            self.bridge.register_channels(channels);
            self.tool_dispatcher.register_tools(tools);
        }
    }
}
```

### 3.2 BridgeManager

负责连接 channel adapter 和 kernel：

```
消息流:
  用户 → Channel → OCMessageCallback → BridgeManager → AgentRouter → Agent
  Agent 回复 → BridgeManager → oc_channel_send() → Channel → 用户
```

- 从 channel 接收 `OCMessage`，转为 kernel 的消息格式
- 携带 `tenant_id` + `sender_id` 到 agent session 上下文
- agent 回复时，从上下文取回 channel + tenant_id，调用 `oc_channel_send()`

### 3.3 ToolDispatcher

管理插件注册的 tools：

- 维护 `tool_name → (plugin_handle, tool_name)` 的映射
- agent 调用 tool 时，查找映射 → 调用 `oc_plugin_tool_execute()`
- 传入 `context_json`：包含 `tenant_id`（从消息上下文获取）、`sender_id`、`agent_id`
- 插件根据 `tenant_id` 选择对应 token 执行

### 3.4 消息上下文中的 tenant_id

tenant_id 从 channel 消息流进 agent session，跟着整个调用链走：

```
OCMessage { tenant_id: "corp_A", sender_id: "user_123" }
  → BridgeManager.dispatch(message)
    → kernel.send_message(agent_id, text, sender_id, sender_name, channel_meta)
      → agent session 上下文 { tenant_id: "corp_A", sender_id: "user_123" }
        → agent 调用 create_spreadsheet
          → tool_dispatcher.execute("create_spreadsheet", args, context)
            → context = { tenant_id: "corp_A", sender_id: "user_123", agent_id }
              → 插件内部: token = tokens[tenant_id]
              → 调用企微 API (使用 corp_A 的 token)
```

主程序需要在 kernel 的 `send_message` 链路中增加 `channel_meta: HashMap<String, String>` 字段，传递 tenant_id 等上下文信息。

## 4. 插件侧实现

### 4.1 插件项目结构

```
opencarrier-plugin-wecom/
  Cargo.toml
  src/
    lib.rs          — 导出 oc_* 函数，实现 ABI
    channel/
      mod.rs        — Channel 实现
      wecom_ws.rs   — 企微 AI 机器人 WebSocket
      wecom_app.rs  — 企微应用 Webhook
      wecom_kf.rs   — 微信客服
    tools/
      mod.rs        — Tool 注册
      wedoc.rs      — 企微文档工具
      message.rs    — 发送应用消息
    token.rs        — Token 管理（多租户）
    crypto.rs       — AES 加解密
  plugin.toml       # 插件元数据 + 配置
```

### 4.2 Cargo.toml

```toml
[package]
name = "opencarrier-plugin-wecom"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
opencarrier-plugin-sdk = "0.1"   # SDK：提供 Rust 友好的 ABI 封装
reqwest = { version = "0.12", features = ["json"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
async-trait = "0.1"
tracing = "0.1"
sha1 = "0.10"
aes = "0.8"
cbc = "0.1"
hex = "0.4"
base64 = "0.22"
roxmltree = "0.10"
```

### 4.3 插件 SDK（opencarrier-plugin-sdk）

提供 Rust 友好的封装，让插件开发者不需要直接写 `extern "C"`：

```rust
// opencarrier-plugin-sdk
pub use opencarrier_types::plugin::*;  // 共享类型

/// 插件开发者实现这个 trait
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    /// 初始化，传入配置
    fn init(config: PluginConfig, context: PluginContext) -> Result<Self, PluginError>
    where Self: Sized;

    /// 返回提供的 channels
    fn channels(&self) -> Vec<Box<dyn ChannelAdapter>>;

    /// 返回提供的 tools
    fn tools(&self) -> Vec<Box<dyn ToolProvider>>;

    /// 停止
    fn stop(&self);
}

/// Channel adapter trait
pub trait ChannelAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn channel_type(&self) -> &str;
    fn start(&self, sender: Box<dyn MessageSender>) -> Result<(), PluginError>;
    fn send(&self, tenant_id: &str, user_id: &str, content: ChannelContent) -> Result<(), PluginError>;
    fn stop(&self);
}

/// Tool provider trait
pub trait ToolProvider: Send + Sync {
    fn definition(&self) -> ToolDef;
    fn execute(&self, args: &serde_json::Value, context: &ToolContext) -> Result<String, PluginError>;
}

/// 工具执行上下文
pub struct ToolContext {
    pub tenant_id: String,
    pub sender_id: String,
    pub agent_id: String,
    pub channel_type: String,
}

/// SDK 提供的宏，自动生成 C ABI 导出
#[macro_export]
macro_rules! declare_plugin {
    ($plugin_type:ty) => {
        #[no_mangle]
        pub extern "C" fn oc_plugin_name() -> *const c_char { ... }
        #[no_mangle]
        pub extern "C" fn oc_plugin_init(...) -> *mut OCPlugin { ... }
        // ... 自动生成所有 oc_* 函数
    };
}
```

插件开发者只需：

```rust
struct WeComPlugin {
    config: WeComConfig,
    tokens: TokenManager,
}

impl Plugin for WeComPlugin {
    fn init(config: PluginConfig, context: PluginContext) -> Result<Self> {
        let wecom_config: WeComConfig = config.parse()?;
        let tokens = TokenManager::new(&wecom_config.tenants);
        Ok(Self { config: wecom_config, tokens })
    }

    fn channels(&self) -> Vec<Box<dyn ChannelAdapter>> {
        vec![
            Box::new(WecomAppChannel::new(self.tokens.clone(), ...)),
        ]
    }

    fn tools(&self) -> Vec<Box<dyn ToolProvider>> {
        vec![
            Box::new(CreateSpreadsheetTool::new(self.tokens.clone())),
            Box::new(AddRowsTool::new(self.tokens.clone())),
            Box::new(SendAppMessageTool::new(self.tokens.clone())),
        ]
    }
}

declare_plugin!(WeComPlugin);
```

## 5. 多租户 Token 管理

```
TokenManager (插件内部)
  ├── tenants: HashMap<String, TenantTokens>
  │     ├── "corp_A" → TenantTokens {
  │     │       corp_token: (value, expiry),
  │     │       suite_ticket: "...",
  │     │   }
  │     ├── "corp_B" → TenantTokens { ... }
  │     └── "corp_C" → TenantTokens { ... }
  │
  └── get_token(tenant_id) → Result<String>
        1. 查缓存，未过期直接返回
        2. 过期则刷新（调用 qyapi.weixin.qq.com）
        3. 失败则返回错误
```

- channel 和 tools 共享同一个 `TokenManager` 实例
- `ToolContext.tenant_id` 传入 → `tokens.get(tenant_id)` → 执行 API 调用
- 分身不知道 token 的存在

## 6. Hub 协议

### 6.1 插件发布

```
POST /api/plugins/publish
Content-Type: multipart/form-data

metadata: {
    name: "wecom",
    version: "1.0.0",
    description: "企业微信集成",
    author: "developer",
    min_opencarrier_version: "0.1.0",
    abi_version: 1,
    channels: ["wecom"],
    tools: ["create_spreadsheet", "add_rows", ...],
    source_type: "source",       // "source" | "binary"
    source_os: null,             // binary 时: "macos" | "linux" | "windows"
    source_arch: null,           // binary 时: "x86_64" | "aarch64"
}
file: wecom-v1.0.0.agx          # 源码包或预编译二进制
signature: <签名>
```

### 6.2 插件发现

```
GET /api/plugins?q=wecom&os=macos&arch=aarch64

[
  {
    name: "wecom",
    version: "1.0.0",
    description: "企业微信集成",
    downloads: 128,
    rating: 4.8,
    channels: ["wecom"],
    tools: ["create_spreadsheet", "add_rows", ...],
  }
]
```

### 6.3 插件下载

```
GET /api/plugins/wecom/1.0.0/download?os=macos&arch=aarch64

→ 返回 .agx 文件流
```

## 7. CLI 命令

```bash
# 搜索插件
opencarrier plugin search wecom

# 安装插件（源码包 → 编译）
opencarrier plugin install wecom

# 安装指定版本
opencarrier plugin install wecom@1.0.0

# 列出已安装插件
opencarrier plugin list

# 配置插件（交互式）
opencarrier plugin config wecom

# 卸载插件
opencarrier plugin uninstall wecom

# 更新插件
opencarrier plugin update wecom
```

## 8. Dashboard 页面

新增 **插件** 标签页：
- 已安装插件列表（状态、channel、tools）
- 插件市场（搜索、浏览、安装）
- 插件配置表单（根据 plugin.toml 的 config_schema 渲染）
- 插件日志查看

## 9. 修改文件清单

| 文件 | 改动 |
|------|------|
| `crates/opencarrier-types/src/plugin.rs` | **新建**：plugin ABI 类型（ChannelContent、ToolDef、PluginConfig 等） |
| `crates/opencarrier-types/src/lib.rs` | 添加 `pub mod plugin` |
| `crates/opencarrier-runtime/src/plugin/mod.rs` | **新建**：plugin 模块入口 |
| `crates/opencarrier-runtime/src/plugin/loader.rs` | **新建**：dlopen 加载 + ABI 绑定 |
| `crates/opencarrier-runtime/src/plugin/manager.rs` | **新建**：插件生命周期管理 |
| `crates/opencarrier-runtime/src/plugin/bridge.rs` | **新建**：连接 channel → kernel 的消息桥 |
| `crates/opencarrier-runtime/src/plugin/tool_dispatch.rs` | **新建**：插件 tool 分发 |
| `crates/opencarrier-runtime/src/tool_runner.rs` | 集成 plugin tool 执行路径 |
| `crates/opencarrier-runtime/src/lib.rs` | 添加 `pub mod plugin` |
| `crates/opencarrier-kernel/src/kernel.rs` | send_message 链路增加 channel_meta |
| `crates/opencarrier-cli/src/main.rs` | plugin 子命令 |
| `crates/opencarrier-api/src/routes.rs` | plugin API 路由 |
| `crates/opencarrier-api/src/server.rs` | 启动时加载插件 |
| `sdk/rust/opencarrier-plugin-sdk/` | **新建**：插件开发 SDK |

## 10. 分阶段实施

### Phase 1: 插件基础设施
- 定义 plugin ABI 类型（opencarrier-types）
- 实现 PluginLoader（dlopen + 符号加载）
- 实现 BridgeManager（channel → kernel）
- 实现 ToolDispatcher（tool 注册 + 执行）
- 基础 CLI 命令（plugin list / install / uninstall）
- 手动测试：写一个 hello-world 插件验证流程

### Phase 2: 插件开发 SDK
- opencarrier-plugin-sdk crate
- `declare_plugin!` 宏
- ChannelAdapter / ToolProvider trait 封装
- 示例插件 + 开发文档

### Phase 3: WeCom 插件
- opencarrier-plugin-wecom 独立项目
- 实现 channel（wecom_app / wecom_kf）
- 实现 tools（wedoc / send_message）
- 多租户 token 管理
- 实际 API 调试

### Phase 4: Hub 集成
- Hub 插件发布/下载 API
- 插件市场 dashboard 页面
- 源码包编译安装流程
- 签名验证

### Phase 5: 生态完善
- 更多内置插件模板
- 插件开发指南
- 插件版本管理 + 自动更新
- 插件权限沙箱
