# OpenCarrier 开发计划

> 基于 OpenCarrier 改造，实现 yingheclient 的 Rust 版本

## 项目概述

| 项目 | 语言 | 状态 | 说明 |
|------|------|------|------|
| yingheclient | TypeScript | 生产环境 | 保持不动，继续服务 |
| opencarrier | Rust | Phase 3 进行中 | 基于 OpenCarrier 改造，已实现 CLI 基础、云端绑定、Relay 客户端框架 |

**目标**: 功能对等的 Rust 实现，命令统一为 `yinghe`

**当前进度**: Phase 0 ✅ | Phase 1 ✅ | Phase 2 ✅ | Phase 3 ⏳ 待开始

---

## 核心差异对比

### yingheclient 功能需求

| 功能 | yingheclient 实现 | OpenCarrier 对应 | 改造策略 |
|------|-------------------|---------------|----------|
| LLM 调用 | ProxyLLM (云端代理) | 多驱动 | 新增 ProxyLLM driver |
| 工具执行 | Agent Tools | tool_runner | ✅ 直接复用 |
| 沙箱 | Worker Thread | WASM + subprocess | ✅ 更安全，直接复用 |
| Skill | .skill 文件 + zip | Skills + Hands | 格式适配 |
| 连接 | Relay WebSocket | channels | 新增 Relay channel |
| 会话 | SessionManager | kernel session | ✅ 直接复用 |
| 调度 | 定时任务 | scheduler | ✅ 直接复用 |
| 记忆 | memory/ | opencarrier-memory | ✅ 直接复用 |
| 加密 | Ed25519 + AES | crypto | 需要适配 |

### 命令对照

```bash
# yingheclient 命令
yinghe serve           # serve 模式 (stdin/stdout)
yinghe status          # 查看状态
yinghe bind <code>     # 绑定
yinghe unbind          # 解绑

# opencarrier 目标命令 (保持一致)
yinghe serve
yinghe status
yinghe bind <code>
yinghe unbind
```

---

## 架构改造

### Crate 重命名

```
opencarrier-runtime    → opencarrier-runtime
opencarrier-kernel     → opencarrier-kernel
opencarrier-cli        → opencarrier-cli (→ yinghe 二进制)
opencarrier-types      → opencarrier-types
opencarrier-memory     → opencarrier-memory
opencarrier-skills     → opencarrier-skills
opencarrier-api        → opencarrier-api
opencarrier-channels   → opencarrier-channels
opencarrier-wire       → 删除 (不需要 P2P)
opencarrier-hands      → 删除 (不需要)
opencarrier-extensions → 删除 (不需要)
opencarrier-desktop    → 删除 (不需要)
opencarrier-migrate    → 删除 (不需要)
```

### 新增模块

```
crates/
├── ying-relay/           # Relay WebSocket 连接
│   ├── src/
│   │   ├── client.rs     # WebSocket 客户端
│   │   ├── protocol.rs   # 消息协议
│   │   ├── auth.rs       # 认证逻辑
│   │   └── reconnect.rs  # 重连机制
│   └── Cargo.toml
│
└── ying-driver/          # ProxyLLM 驱动
    ├── src/
    │   ├── proxy.rs      # 云端代理调用
    │   └── types.rs      # 请求/响应类型
    └── Cargo.toml
```

---

## 开发阶段

### Phase 0: 项目初始化 (1-2 天) ✅ 完成

- [x] 复制 OpenCarrier 代码到 yinnhoos/opencarrier
- [x] 重命名核心 crates (opencarrier → opencarrier)
- [x] 更新根 Cargo.toml
- [x] 删除不需要的 crates (desktop)
- [x] 创建 stub crates (wire, hands, extensions, migrate)
- [x] 完善 stub 类型定义以匹配 kernel 使用
- [x] 编译验证通过
- [ ] 设置 CI/CD

**验证状态** (2026-03-20):
- `cargo build --workspace` ✅ 编译通过
- `cargo test --workspace` ✅ 419 passed, 1 failed (wecom 硬编码字符串)
- `yinghe --version` ✅ 输出 `opencarrier 0.1.0`
- `yinghe status` ✅ 正常工作
- `yinghe bind` ✅ 生成配对码

**已创建的 stub crates**:
```
crates/
├── opencarrier-wire/      # P2P 通信 (完整实现)
├── opencarrier-hands/     # Hands 系统 (完整实现，8 个内置 hands)
├── opencarrier-extensions/ # 扩展系统 (完整实现，25 templates)
└── opencarrier-migrate/   # 迁移工具 (完整实现)
```

### Phase 1: CLI 基础 (2-3 天) ✅ 完成

- [x] 修改 CLI 入口为 `yinghe` 命令
- [x] 实现 `yinghe serve` 子命令 (stdin/stdout 模式) - 通过 `yinghe start` 实现
- [x] 实现 `yinghe status` 子命令
- [x] 实现 `yinghe bind <code>` 子命令
- [x] 实现 `yinghe unbind` 子命令
- [x] 配置文件路径: `~/.opencarrier/config.toml`

**验证状态**:
- `yinghe --help` ✅ 显示完整命令列表
- `yinghe status` ✅ 显示 agent 状态、provider、model、数据目录
- `yinghe bind` ✅ 生成 6 位配对码，等待 App 绑定
- `yinghe unbind` ✅ 解除云端绑定
- 配对码功能: ✅ 云端 `cloud_client` 模块正常工作

### Phase 2: Relay 连接 (3-5 天)

- [x] 创建 ying-relay crate
- [x] WebSocket 客户端实现
- [x] 消息协议 (JSON)
- [x] Ed25519 认证
- [x] 心跳保活
- [x] 断线重连
- [x] 与 yingheclient 协议兼容
- [x] 集成到 opencarrier-runtime ✅
- [x] 连接 relay WebSocket 报告在线状态 ✅

**参考**: `yingheclient/src/connection/relay-connection.ts`

**已创建文件**:
```
crates/ying-relay/
├── src/
│   ├── lib.rs       # 模块导出
│   ├── client.rs    # WebSocket 客户端
│   ├── protocol.rs  # 消息协议
│   ├── auth.rs     # Ed25519 认证
│   └── crypto.rs   # 加密/解密
└── Cargo.toml
```

### Phase 3: ProxyLLM Driver (2-3 天)

- [ ] 创建 ying-driver crate
- [ ] 实现 ProxyLLM driver
- [ ] 模态配置 (modality)
- [ ] 工具调用支持
- [ ] 集成到 opencarrier-runtime

**参考**: `yingheclient/src/llm/client/proxy-client.ts`

### Phase 4: Skill 系统 (3-5 天)

- [ ] 适配 yingheclient Skill 格式
- [ ] Skill 加载 (.skill 文件 + zip)
- [ ] Skill 签名验证
- [ ] Skill 执行 (沙箱)
- [ ] Plugin 适配

**参考**: `yingheclient/src/skill/`, `yingheclient/src/plugin/`

### Phase 5: 会话管理 (2-3 天)

- [ ] 会话持久化
- [ ] 消息格式适配
- [ ] 多轮对话
- [ ] 上下文管理

**参考**: `yingheclient/src/conversation/`

### Phase 6: 集成测试 (3-5 天)

- [ ] agentd 集成测试
- [ ] 与 TypeScript 版本对比测试
- [ ] 性能测试
- [ ] 内存泄漏检测

### Phase 7: 部署 (1-2 天)

- [ ] 编译优化 (release)
- [ ] 打包脚本
- [ ] 文档更新

---

## 关键文件参考

### yingheclient 核心文件

```
yingheclient/src/
├── cli/index.ts              # CLI 入口
├── connection/
│   └── relay-connection.ts   # Relay 连接
├── llm/client/
│   └── proxy-client.ts       # ProxyLLM 客户端
├── skill/                    # Skill 系统
├── plugin/                   # Plugin 系统
├── conversation/             # 会话管理
├── task-system/              # 任务系统
└── sandbox/                  # 沙箱
```

### OpenCarrier 对应模块

```
opencarrier/crates/
├── opencarrier-cli/          # CLI
├── opencarrier-runtime/      # 运行时
│   ├── drivers/              # LLM 驱动
│   ├── sandbox/              # 沙箱
│   └── tool_runner/          # 工具执行
├── opencarrier-kernel/       # 内核
├── opencarrier-skills/       # 技能
└── opencarrier-memory/       # 记忆
```

---

## 消息协议

### serve 模式 (stdin/stdout)

**输入**:
```json
{
  "type": "chat",
  "conversationId": "conv-123",
  "conversationType": "carrier",
  "chatType": "direct",
  "content": "你好"
}
```

**输出**:
```json
{
  "type": "chat_response",
  "conversationType": "carrier",
  "chatType": "direct",
  "response": "你好！有什么可以帮你的？",
  "metadata": {
    "rounds": 1,
    "toolCalls": 0
  }
}
```

### Relay 协议

保持与 yingheclient 相同的 WebSocket 消息格式。

---

## 验收标准

1. **功能对等**: 所有 yingheclient 功能都能在 opencarrier 中实现
2. **协议兼容**: 与 agentd、Relay 服务器协议完全兼容
3. **命令一致**: `yinghe serve/status/bind/unbind` 命令行为一致
4. **性能提升**: 启动时间 < 100ms，内存占用 < 50MB
5. **测试覆盖**: 核心模块测试覆盖率 > 80%

---

## 时间估算

| 阶段 | 时间 | 累计 |
|------|------|------|
| Phase 0: 初始化 | 1-2 天 | 2 天 |
| Phase 1: CLI | 2-3 天 | 5 天 |
| Phase 2: Relay | 3-5 天 | 10 天 |
| Phase 3: ProxyLLM | 2-3 天 | 13 天 |
| Phase 4: Skill | 3-5 天 | 18 天 |
| Phase 5: 会话 | 2-3 天 | 21 天 |
| Phase 6: 测试 | 3-5 天 | 26 天 |
| Phase 7: 部署 | 1-2 天 | 28 天 |

**总计: 3-4 周**

---

## 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| OpenCarrier 代码复杂 | 高 | 先熟悉架构，逐步改造 |
| 协议不兼容 | 高 | 参考 yingheclient 逐字段对齐 |
| Skill 格式差异 | 中 | 编写转换层 |
| 编译问题 | 中 | 保持增量编译，及时测试 |

---

## 下一步

1. Phase 2: 实现 Relay 连接 (WebSocket)
2. Phase 3: 实现 ProxyLLM Driver
3. Phase 4: 适配 Skill 系统
4. Phase 5: 会话管理
5. Phase 6: 集成测试
6. Phase 7: 部署
