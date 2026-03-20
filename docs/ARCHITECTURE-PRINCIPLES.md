# OpenCarrier 架构原则

> **版本**: v1.0
> **日期**: 2026-03-20
> **状态**: 已确立

---

## 1. 核心架构：App 是大脑，Carrier 是双手

### 1.1 职责划分

```
┌─────────────────────────────────────────────────────────────┐
│                         App 端                               │
│                       (大脑/Brain)                           │
│                                                              │
│  • 记忆管理 (Memory Management)                               │
│    - Session 会话持久化                                       │
│    - 上下文管理                                               │
│    - 记忆压缩 (发起方)                                        │
│                                                              │
│  • 协调调度 (Coordination)                                    │
│    - 多 Agent 协调                                           │
│    - 任务分发                                                 │
│    - 状态监控                                                 │
│                                                              │
│  • 用户交互 (User Interaction)                                │
│    - 消息收发                                                 │
│    - 界面展示                                                 │
│    - 通知推送                                                 │
└─────────────────────────────────────────────────────────────┘
                              │
                              │ A2A Protocol (JSON-RPC over TCP)
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                       Carrier 端                              │
│                       (双手/Hands)                            │
│                                                              │
│  • 任务执行 (Task Execution)                                  │
│    - 任务分解 (Decomposition)                                 │
│    - 工具调用 (Tool Invocation)                               │
│    - 结果保证 (Guaranteed Results)                            │
│                                                              │
│  • 执行监控 (Execution Monitoring)                            │
│    - 进度跟踪                                                 │
│    - 错误检测                                                 │
│    - 自动修复 (Auto-repair)                                   │
│                                                              │
│  • LLM 推理 (LLM Inference)                                   │
│    - 上下文理解                                               │
│    - 决策生成                                                 │
│    - 响应构建                                                 │
└─────────────────────────────────────────────────────────────┘
```

### 1.2 关键原则

| 原则 | 说明 |
|------|------|
| **App 拥有记忆** | 所有会话历史、用户偏好、上下文信息存储在 App 端 |
| **Carrier 无状态** | Carrier 不持久化记忆，每次请求独立处理 |
| **压缩由 App 发起** | 当记忆超过阈值时，App 发起压缩请求，Carrier 执行压缩 |
| **执行结果保证** | Carrier 负责确保任务完成，包括重试和修复 |

---

## 2. 记忆管理原则

### 2.1 记忆归属

```
记忆存储位置：App 端

原因：
1. App 是用户入口，需要维护完整的对话上下文
2. 一个用户可能连接多个 Carrier，统一在 App 管理
3. Carrier 可能随时下线，记忆不能丢失
4. 压缩策略由 App 根据实际需求决定
```

### 2.2 记忆压缩流程

```
┌─────────┐                    ┌───────────┐
│   App   │                    │  Carrier  │
└────┬────┘                    └─────┬─────┘
     │                               │
     │ 1. 检测记忆大小 > 阈值         │
     │                               │
     │ 2. 发送压缩请求               │
     │   {                          │
     │     method: "compactMemory", │
     │     messages: [...],         │
     │     keepRecent: 50           │
     │   }                          │
     │──────────────────────────────▶│
     │                               │
     │                               │ 3. LLM 生成摘要
     │                               │
     │ 4. 返回压缩结果               │
     │   {                          │
     │     summary: "...",          │
     │     recentMessages: [...]    │
     │   }                          │
     │◀──────────────────────────────│
     │                               │
     │ 5. 存储压缩后的记忆           │
     │                               │
```

### 2.3 压缩策略

```typescript
// App 端的压缩配置
interface MemoryConfig {
  // 触发压缩的消息数量阈值
  compactThreshold: number;  // 默认 100

  // 保留的最近消息数量
  keepRecentWindow: number;  // 默认 50

  // 压缩后格式
  // summary: LLM 生成的摘要
  // recentMessages: 最近 N 条原始消息
}
```

---

## 3. 任务执行原则

### 3.1 Carrier 的职责

```rust
/// Carrier 的核心能力
trait CarrierCapabilities {
    /// 1. 任务分解：将大任务拆分成可执行的小任务
    fn decompose(&self, task: Task) -> Vec<SubTask>;

    /// 2. 工具执行：调用工具完成任务
    fn execute(&self, subtask: SubTask) -> Result<Output>;

    /// 3. 执行监控：跟踪任务进度，检测异常
    fn monitor(&self, execution: Execution) -> Status;

    /// 4. 自动修复：遇到错误时自动重试或调整策略
    fn repair(&self, error: Error) -> RecoveryPlan;

    /// 5. 结果保证：确保最终返回有效结果
    fn guarantee(&self, result: Result) -> GuaranteedResult;
}
```

### 3.2 执行保证机制

```
任务执行流程：

输入任务
    │
    ▼
┌─────────────────┐
│  任务分解       │ ← 拆分成可并行的子任务
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│  并行执行       │ ← 调用工具，执行子任务
└────────┬────────┘
         │
         ▼
┌─────────────────┐     ┌─────────────────┐
│  结果检查       │────▶│  成功？          │
└─────────────────┘     └────────┬────────┘
                                 │
                    ┌────────────┼────────────┐
                    │ No         │            │ Yes
                    ▼            │            ▼
           ┌─────────────────┐   │   ┌─────────────────┐
           │  错误分析       │   │   │  汇总结果       │
           └────────┬────────┘   │   └────────┬────────┘
                    │            │            │
                    ▼            │            │
           ┌─────────────────┐   │            │
           │  自动修复       │   │            │
           │  (重试/调整)    │───┘            │
           └─────────────────┘                │
                                               ▼
                                      ┌─────────────────┐
                                      │  返回保证结果   │
                                      └─────────────────┘
```

---

## 4. 通信协议

### 4.1 A2A 协议

- **传输层**: JSON-RPC 2.0 over TCP
- **默认端口**: 86
- **消息格式**: 每行一个 JSON 对象，以 `\n` 结尾

### 4.2 主要方法

| 方法 | 发起方 | 说明 |
|------|--------|------|
| `sendMessage` | App → Carrier | 发送消息给 Agent |
| `compactMemory` | App → Carrier | 发起记忆压缩 |
| `getAgentCard` | App → Carrier | 获取 Agent 能力 |
| `executeTask` | App → Carrier | 执行任务 |
| `taskStatus` | Carrier → App | 任务状态更新 |
| `taskResult` | Carrier → App | 任务执行结果 |

---

## 5. 开发规范

### 5.1 App 端开发

```kotlin
// App 端职责清单
class AppResponsibilities {
    // ✅ 应该做
    - 管理会话历史和上下文
    - 决定何时压缩记忆
    - 协调多个 Carrier
    - 处理用户界面

    // ❌ 不应该做
    - 直接执行复杂任务逻辑
    - 存储业务数据（交给 Carrier 或后端）
    - 长时间阻塞等待
}
```

### 5.2 Carrier 端开发

```rust
// Carrier 端职责清单
struct CarrierResponsibilities {
    // ✅ 应该做
    - 任务分解和执行
    - 工具调用和管理
    - 错误检测和自动修复
    - 保证返回有效结果

    // ❌ 不应该做
    - 持久化用户会话记忆
    - 决定压缩策略
    - 跨会话保持状态（除非明确要求）
}
```

---

## 6. 参考文档

- [A2A 协议应用及开发计划](./A2A-PROTOCOL-PLAN.md)
- [Agent Protocol 规范](../../agent-cli/docs/PROTOCOL.md)
- [JSON-RPC 2.0 Specification](https://www.jsonrpc.org/specification)

---

**最后更新**: 2026-03-20
**维护者**: 应合网络团队
