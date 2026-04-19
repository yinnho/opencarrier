# Brain 系统 — 设计与实现

## 概述

Brain 是 OpenCarrier 的 LLM 路由大脑。它从 `brain.json` 读取用户配置，在启动时预创建所有 endpoint driver，运行时按 modality（任务类型）路由 LLM 请求，并提供 fallback 链、健康追踪和热重载。

```
brain.json
    │
    ▼
┌─────────────────────────────────────────┐
│  Brain (kernel crate)                    │
│                                          │
│  providers ──► endpoints ──► modalities  │
│  (身份+凭证)   (可调用单元)    (任务路由)   │
│       │            │             │       │
│       │      drivers HashMap    │       │
│       │      (预创建)           │       │
│       │            │             │       │
│       │      health DashMap     │       │
│       │      (原子追踪)         │       │
└────────┼────────────┼────────────┼───────┘
         │            │            │
         ▼            ▼            ▼
    credentials   LlmDriver    endpoints_for()
    (skill注入)   (complete/    (agent_loop
                  stream)       fallback链)
```

---

## 三层架构

### 第一层：Provider（身份 + 凭证）

```json
{
  "zhipu": { "api_key_env": "ANTHROPIC_API_KEY" },
  "ollama": {},
  "kling": {
    "params": {
      "access_key_env": "KLING_ACCESS_KEY",
      "secret_key_env": "KLING_SECRET_KEY"
    }
  }
}
```

Provider 只知道"我是谁"和"怎么认证"。不包含 URL、模型、协议格式。

- `api_key_env`: 环境变量名（不是 key 本身），运行时 `std::env::var()` 读取
- `params`: 多凭证 provider 的额外参数（key = 逻辑名, value = 环境变量名）
- 如果 `api_key_env` 为空，表示不需要认证（如本地 Ollama）

**代码位置**: `opencarrier-types/src/brain.rs` → `ProviderConfig`

### 第二层：Endpoint（完整可调用单元）

```json
{
  "zhipu_anthropic": {
    "provider": "zhipu",
    "model": "glm-5.1",
    "base_url": "https://open.bigmodel.cn/api/anthropic",
    "format": "anthropic"
  },
  "ollama_local": {
    "provider": "ollama",
    "model": "llama3.2:latest",
    "base_url": "http://localhost:11434/v1"
  }
}
```

Endpoint 包含发起一次 LLM API 调用所需的全部信息：
- `provider`: 从哪获取凭证
- `model`: 请求哪个模型
- `base_url`: 发到哪
- `format`: 用什么协议（`openai` / `anthropic` / `gemini`）

`format` 决定使用哪个 driver：
- `openai` → OpenAIDriver（同时兼容 Ollama/vLLM/Groq/DeepSeek 等）
- `anthropic` → AnthropicDriver
- `gemini` → GeminiDriver

**代码位置**: `opencarrier-types/src/brain.rs` → `EndpointConfig`

### 第三层：Modality（任务类型 → Endpoint 路由）

```json
{
  "chat": {
    "primary": "zhipu_anthropic",
    "fallbacks": ["deepseek_chat"],
    "description": "主力对话"
  },
  "fast": {
    "primary": "ollama_local",
    "description": "本地快速推理"
  }
}
```

Modality 将任务类型映射到 endpoint：
- `primary`: 首选 endpoint
- `fallbacks`: 失败后的备选列表（按顺序尝试）
- `description`: 人类可读描述

**代码位置**: `opencarrier-types/src/brain.rs` → `ModalityConfig`

---

## 完整数据流

### 1. 启动初始化

```
Kernel::boot()
  → 读 config.toml: brain.config = "brain.json"
  → 读 ~/.opencarrier/brain.json → BrainConfig
  → Brain::new(config)
    → 遍历 endpoints，为每个创建 driver:
        create_driver(name, endpoint, providers)
          → 查找 provider → 读取 api_key_env → 构建 DriverConfig
          → drivers::create_driver(config) → Arc<dyn LlmDriver>
          → 失败的 endpoint 记录警告但继续（不阻塞启动）
    → 校验 modalities 引用的 endpoint 都有 driver
    → 存入: drivers HashMap, health DashMap
  → 存入 kernel.brain: Arc<RwLock<Arc<Brain>>>
```

**关键设计**：
- Driver 创建失败不会阻止启动（只跳过该 endpoint）
- 但如果所有 endpoint 都失败 → 返回 `BrainError::NoEndpoints`
- Triple-wrapping `Arc<RwLock<Arc<Brain>>>` 支持热重载：写入时原子替换 Arc

### 2. 运行时调用（agent_loop）

```
agent_loop 收到用户消息
  → 确定使用哪个 modality（默认 "chat"）
  → call_with_fallback(brain, fallback_driver, modality, request)
    → brain.endpoints_for(modality)
        返回 [primary, fallback1, fallback2, ...] 的 ResolvedEndpoint 列表
    → 遍历每个 endpoint:
        brain.driver_for_endpoint(ep.id) → Arc<dyn LlmDriver>
        → 设置 request.model = ep.model
        → call_with_retry(driver, request)
            → 检查 ProviderCooldown 断路器
            → driver.complete(request)
            → 失败时自动重试（rate-limit / overload）
        → 成功: brain.report(success) → return response
        → 失败: brain.report(failure) → 尝试下一个 endpoint
    → 所有 endpoint 都失败 → 返回错误
```

**关键设计**：
- `call_with_fallback` 是统一入口，同时支持 streaming（`stream_with_fallback`）
- `call_with_retry` 处理单次调用的重试逻辑（rate-limit、overload）
- 每次调用后通过 `brain.report()` 反馈结果
- Fallback 是串行的（不是并行竞速）

### 3. 健康反馈

```
brain.report(EndpointReport { endpoint_id, success, latency_ms, error })
  → EndpointTracker (lock-free atomics):
    success → success_count++, consecutive_failures = 0
    failure → failure_count++, consecutive_failures++
  → 查询时: snapshot() → (success, failure, avg_latency, consecutive_failures)
```

**EndpointTracker 内部结构**（全原子操作，无锁）：
- `success_count: AtomicU64` — 成功次数
- `failure_count: AtomicU64` — 失败次数
- `total_latency_ms: AtomicU64` + `latency_count: AtomicU64` — 计算平均延迟
- `consecutive_failures: AtomicU32` — 连续失败次数

### 4. 热重载

```
用户调用 PUT /api/brain/... 修改配置
  → routes::update_brain_provider() / update_brain_endpoint() / ...
    → kernel.update_brain(|config| { ... 修改 config ... })
      → 读当前 config（RwLock read）
      → 应用修改
      → 写回 brain.json
      → Brain::new(new_config) → 创建新 Brain
      → RwLock write → 替换 Arc<Brain>
```

或手动触发：

```
POST /api/brain/reload
  → kernel.reload_brain()
    → 从 brain.json 重新读取
    → Brain::new(config)
    → RwLock write → 替换 Arc<Brain>
```

---

## Brain Trait（运行时接口）

定义在 `opencarrier-runtime/src/llm_driver.rs`，由 `opencarrier-kernel/src/brain.rs` 实现。

```rust
#[async_trait]
pub trait Brain: Send + Sync {
    // --- 查询接口 ---
    fn list_modalities(&self) -> Vec<ModalityInfo>;
    fn endpoints_for(&self, modality: &str) -> Vec<ResolvedEndpoint>;
    fn driver_for_endpoint(&self, endpoint_id: &str) -> Option<Arc<dyn LlmDriver>>;
    fn report(&self, report: EndpointReport);
    fn status(&self) -> BrainStatus;
    fn credentials_for(&self, provider: &str) -> Option<ProviderCredentials>;

    // --- Legacy ---
    fn model_for(&self, modality: &str) -> &str;
    fn has_modality(&self, modality: &str) -> bool;
}
```

**设计原则**：Brain 是纯查询服务，不执行 LLM 调用。执行和 fallback 逻辑在 `agent_loop` 的 `call_with_fallback` / `stream_with_fallback` 中。

---

## API 端点

| 方法 | 路径 | 功能 |
|------|------|------|
| GET | `/api/brain` | Brain 配置概要（providers, endpoints, modalities） |
| GET | `/api/brain/status` | 健康状态（driver 就绪数、延迟、成功/失败计数） |
| GET | `/api/brain/modalities/{name}` | 单个 modality 的 resolved endpoint 链 |
| PUT | `/api/brain/providers/{name}` | 创建/更新 provider |
| DELETE | `/api/brain/providers/{name}` | 删除 provider |
| PUT | `/api/brain/endpoints/{name}` | 创建/更新 endpoint |
| DELETE | `/api/brain/endpoints/{name}` | 删除 endpoint |
| PUT | `/api/brain/modalities/{name}` | 创建/更新 modality |
| DELETE | `/api/brain/modalities/{name}` | 删除 modality |
| PUT | `/api/brain/default-modality` | 设置默认 modality |
| POST | `/api/brain/reload` | 从磁盘重新加载 brain.json |

---

## 文件清单

| 文件 | 职责 |
|------|------|
| `opencarrier-types/src/brain.rs` | 所有类型定义：BrainConfig, ProviderConfig, EndpointConfig, ModalityConfig, ApiFormat, ResolvedEndpoint, EndpointReport, EndpointHealth, BrainStatus, ProviderCredentials |
| `opencarrier-kernel/src/brain.rs` | Brain 实现：driver 创建、健康追踪 (EndpointTracker)、热重载、BrainTrait 实现 |
| `opencarrier-runtime/src/llm_driver.rs` | Brain trait 定义 + LlmDriver trait + CompletionRequest/Response + StreamEvent |
| `opencarrier-runtime/src/agent_loop.rs` | call_with_fallback / stream_with_fallback — 运行时 fallback 逻辑 |
| `opencarrier-kernel/src/kernel.rs` | Brain 初始化 (brain RwLock)、brain_info()、reload_brain()、update_brain() |
| `opencarrier-api/src/routes.rs` | 11 个 Brain API handler |
| `opencarrier-api/src/server.rs` | 路由注册 |
| `opencarrier-types/src/config.rs` | BrainSourceConfig（brain.json 路径配置） |

---

## Driver 创建工厂

`Brain::create_driver()` 根据 `format` 字段选择 driver 类型：

```rust
fn create_driver(name, endpoint, providers) -> Result<Arc<dyn LlmDriver>, BrainError> {
    let provider_config = providers[endpoint.provider];
    let api_key = std::env::var(provider_config.api_key_env);

    // format 决定 driver 类型:
    let driver_provider = match endpoint.format {
        ApiFormat::OpenAI   => endpoint.provider.as_str(),  // 保留原始 provider 名
        ApiFormat::Anthropic => "anthropic",
        ApiFormat::Gemini    => "gemini",
    };

    drivers::create_driver(DriverConfig {
        provider: driver_provider,
        api_key,
        base_url: endpoint.base_url,
        skip_permissions: true,
    })
}
```

OpenAI format 特殊处理：保留原始 provider 名（如 `ollama`, `groq`, `deepseek`），因为 `create_driver` 内部需要区分 key_required 行为。

---

## 错误处理

### 启动时
- Driver 创建失败 → 警告 + 跳过该 endpoint（不阻塞启动）
- 所有 driver 都失败 → `BrainError::NoEndpoints` → 启动失败
- Modality 引用不存在的 endpoint → 警告（运行时才发现）

### 运行时
- 单次调用失败 → `call_with_retry` 自动重试（rate-limit / overload）
- 重试耗尽 → `call_with_fallback` 尝试下一个 endpoint
- 所有 endpoint 都失败 → 返回最后一个错误

### 热重载
- brain.json 格式错误 → reload 失败，保留旧 Brain
- 新 driver 创建失败 → 跳过，其他 driver 正常工作

---

## 已知限制和待讨论

### 1. 断路器（Circuit Breaker）

`EndpointTracker` 追踪 `consecutive_failures`，当连续失败 ≥ 3 次时自动触发断路：

- **CLOSED（正常）**：consecutive_failures < 3，endpoint 正常参与路由
- **OPEN（断路）**：consecutive_failures ≥ 3 且冷却期未过（60s），`endpoints_for()` 跳过该 endpoint
- **HALF-OPEN（试探）**：冷却期过后，允许一次请求通过；成功则关闭断路，失败则重新计时

`report()` 在失败时也记录 latency 和失败时间戳，数据完整。dashboard 通过 `EndpointHealth.circuit_open` 可以看到断路状态。

