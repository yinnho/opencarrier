# OpenCarrier 开发计划

> **版本**: v2.0
> **日期**: 2026-04-11
> **状态**: 进行中

---

## 项目概述

opencarrier 是分身操作系统（Agent OS），用 Rust 实现。每个用户自部署一个 opencarrier 实例，从 Hub 下载分身运行。

**核心模型**：一户一备。不需要多租户。

| 组件 | 说明 |
|------|------|
| opencarrier | Agent OS 运行时（本仓库） |
| openclone-hub | 分身模版仓库（hub.yinnho.cn） |
| openclone-core | 分身训练工具（CLI + 库） |

---

## 已完成的阶段

### Phase 0-7：基础平台 ✅

opencarrier 从 OpenCarrier fork 而来，完成基础改造：

- [x] Phase 0: 项目初始化、crate 重命名
- [x] Phase 1: CLI 基础命令
- [x] Phase 2: Relay 连接
- [x] Phase 3: ProxyLLM Driver
- [x] Phase 4: Skill 系统
- [x] Phase 5: 会话管理
- [x] Phase 6: 集成测试
- [x] Phase 7: 部署优化

### Phase 8：Hub 集成 ✅

opencarrier 从 Hub 下载分身：

- [x] `opencarrier-clone` crate — .agx 加载、转换、安装
- [x] `HubConfig` — Hub URL + API Key 环境变量
- [x] CLI `hub search` / `hub install` 命令
- [x] API Key 认证 + 设备绑定
- [x] 端到端测试通过

### Phase 9：去多租户 ✅

删除所有多租户代码，让 opencarrier 成为干净的单用户 Agent OS：

- [x] 删除 `UserId` / `UserConfig` / `AuthManager` 等多租户类型
- [x] 删除 `owner_user_id` / `user_index` 等用户隔离逻辑
- [x] 简化 router / registry / channel bridge
- [x] 简化 API 端点（install_clone / list_clones）
- [x] CLI 二进制名从 `yinghe` → `opencarrier`
- [x] 2225 tests 通过，零残留引用

---

## 当前阶段

### Phase 10：分身生命周期系统

让分身能**学习、成长、自我维护**。将 openclone 的核心训练能力变为 opencarrier 的平台级能力。

> 详细设计：[CLONE-LIFECYCLE-SYSTEM.md](./CLONE-LIFECYCLE-SYSTEM.md)

**P0 — 核心（让分身能学习）**：

- [ ] 新建 `opencarrier-lifecycle` crate
- [ ] `evolution.rs` — 对话后自动进化（pre-filter + LLM 分析）
- [ ] `version.rs` — 知识版本管理（JSONL 日志）
- [ ] `parsers/chat.rs` — 聊天记录解析（多平台自动检测）
- [ ] 内核集成 — 对话完成后触发进化 hook

**P1 — 维护（让分身保持健康）**：

- [ ] `health.rs` — 知识 lint + heal
- [ ] `bloat.rs` + `compile.rs` — 膨胀控制 + 自动编译
- [ ] `parsers/faq.rs` + `parsers/document.rs` — 更多数据类型
- [ ] 系统工具注册到 tool_runner

**P2 — 生态（让分身反哺 Hub）**：

- [ ] `evaluate.rs` — 分身质量评估
- [ ] `feedback.rs` — 反馈收集 + 匿名化 + 推送 Hub

---

## 关键架构决策

| 决策 | 原因 |
|------|------|
| 单用户（一户一备） | 每个用户自部署，不需要多租户隔离 |
| 平台提供进化/维护能力 | 不是分身 skill，是 OS 级别的基础设施 |
| 新建 lifecycle crate | 关注点分离，不污染现有 crate |
| 复用 openclone 的算法 | 进化/编译/膨胀控制/解析器已在 openclone 验证过 |

---

## 时间线

| 阶段 | 时间 | 状态 |
|------|------|------|
| Phase 0-7: 基础平台 | 2026-03 ~ 2026-03 | ✅ 完成 |
| Phase 8: Hub 集成 | 2026-04 | ✅ 完成 |
| Phase 9: 去多租户 | 2026-04 | ✅ 完成 |
| Phase 10-P0: 生命周期核心 | — | 📋 设计中 |
| Phase 10-P1: 知识维护 | — | 📋 待开始 |
| Phase 10-P2: 生态闭环 | — | 📋 待开始 |

---

## 设计文档

| 文档 | 内容 |
|------|------|
| [ARCHITECTURE-PRINCIPLES.md](./ARCHITECTURE-PRINCIPLES.md) | 架构原则（v2.0 已更新） |
| [architecture.md](./architecture.md) | 技术架构详情 |
| [CLONE-LIFECYCLE-SYSTEM.md](./CLONE-LIFECYCLE-SYSTEM.md) | 分身生命周期系统设计（新） |
| [skill-development.md](./skill-development.md) | Skill 开发指南 |
| [configuration.md](./configuration.md) | 配置参考 |
| [api-reference.md](./api-reference.md) | API 文档 |
