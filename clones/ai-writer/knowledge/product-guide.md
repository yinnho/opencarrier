---
name: Product Guide
source: manual
type: knowledge
description: OpenCarrier & Aginx 产品介绍素材，用于撰写产品相关文章
tags: [product, opencarrier, aginx]
confidence: EXTRACTED
status: active
---

# OpenCarrier & Aginx 产品素材

## OpenCarrier — 开源 Agent 操作系统

OpenCarrier 是一个用 Rust 编写的开源 Agent 操作系统（Agent OS），核心理念是 **App = Brain, Carrier = Hands**：

- **App 端（大脑）**：记忆管理 + 协调调度
- **Carrier 端（双手）**：任务执行 + 监控修复

### 核心特性

- **14 个 Rust crate** 组成，高性能、内存安全
- **分身系统（Clone）**：可创建专业化 Agent，自带知识、技能、人格
- **MCP 协议支持**：原生支持 Model Context Protocol，连接外部工具
- **A2A 协议**：Agent-to-Agent 通信，支持跨实例协作
- **通道插件**：企微、飞书、微信个人号统一管理
- **Web Dashboard**：Alpine.js SPA，可视化管理和监控
- **安全架构**：路径穿越防护、SSRF 防护、能力系统、沙箱隔离

### 技术栈

| 层级 | 技术 |
|------|------|
| 核心 | Rust, Tokio async runtime |
| LLM | Anthropic/OpenAI/Groq/Ollama 多驱动 |
| API | Axum HTTP + WebSocket |
| 存储 | SQLite (rusqlite), TOML 配置 |
| 前端 | Alpine.js SPA |
| 协议 | MCP (stdio/SSE), A2A, OFP P2P |

### GitHub
https://github.com/yinnho/opencarrier

## Aginx — Agent 服务引擎

Aginx 是 OpenCarrier 团队打造的 Agent 服务引擎，专注于将 AI Agent 能力以服务形式交付给企业客户。

### 核心定位
- 面向企业的 Agent 服务化平台
- 帮助企业快速部署和管理 AI Agent
- 支持多场景：客服、内容创作、数据分析、流程自动化

---

- 2026-04-23: 手动创建，基于 OpenCarrier 代码库和项目理解
