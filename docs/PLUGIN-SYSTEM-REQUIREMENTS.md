# OpenCarrier 插件系统 — 需求文档

## 1. 背景

OpenCarrier 是一个 Agent 操作系统，当前所有功能（LLM driver、tool、channel）都编译在主程序中。随着生态发展，第三方开发者需要能为 OpenCarrier 开发扩展，而用户需要能按需安装这些扩展，无需升级主程序。

参考实现：openfang 的 `openfang-channels` crate（40+ 渠道 adapter）和 openclone 的 `tools/wedoc.rs`（企微平台工具）。

## 2. 目标

- 主程序**零修改**加载新能力
- 开发者可以独立开发、发布插件
- 用户可以按需发现、下载、安装插件
- 支持 channel（消息通道）和 tool（平台能力）两种扩展类型
- 一个插件可以同时提供 channel + tools

## 3. 核心需求

### 3.1 插件定义

插件是一个独立的 Rust 项目，编译为共享库（cdylib）：
- macOS: `.dylib`
- Linux: `.so`
- Windows: `.dll`

插件通过约定的 C ABI 入口函数向主程序注册能力。

### 3.2 两种扩展类型

#### Channel（消息通道）
- 收消息：从平台接收用户消息，转为统一的 `ChannelMessage`
- 发消息：将 agent 回复发回平台
- 生命周期：`start()` / `stop()` / `status()`
- 可选能力：typing 指示器、reaction、threaded reply

#### Tool（平台能力工具）
- agent 可调用的平台 API 封装
- 例：`create_spreadsheet`、`send_app_message`、`add_rows`
- 通过 agent 的 `tool_allowlist` 控制可见性
- 工具执行需要平台 credential（token），由插件内部管理

### 3.3 多租户

- 一个插件实例服务多个租户（如多个企业微信企业）
- 插件配置包含 tenant list，每个 tenant 有独立的 credential
- token 管理在插件内部闭环（获取、缓存、刷新）
- **tenant 路由由消息上下文决定，不由分身决定**
  - 用户消息从 channel 进来时，channel adapter 已知道消息属于哪个 tenant
  - 分身是 tenant-agnostic 的，只关心"创建表格"，不关心"为谁创建"
  - 插件根据消息上下文（corp_id / sender_id / channel metadata）选择对应 tenant 的 token

### 3.4 分发方式

#### 方式一：源码包（.agx）
- 开发者上传源码压缩包到 Hub
- 用户下载后，主程序调用 `cargo build` 编译为 cdylib
- 优势：天然解决 ABI 兼容性（同一编译环境）
- 类似现有分身模板的分发机制

#### 方式二：预编译二进制
- 开发者上传已编译的共享库
- Hub 按平台（macOS/Linux/Windows）+ 架构（x86_64/aarch64）分发
- 优势：安装快，不需要编译环境
- 挑战：ABI 兼容性需严格管理

两种方式并存，优先支持源码包。

### 3.5 插件生命周期

```
发现 → 下载 → 编译（源码包） → 安装 → 加载 → 运行 → 卸载
```

- **发现**：用户在 dashboard/CLI 浏览 Hub，或 agent 自动识别需求
- **下载**：从 Hub 拉取插件包
- **编译**：解压 → `cargo build --release` → 生成 cdylib
- **安装**：复制到 `~/.opencarrier/plugins/{name}/`
- **加载**：启动时扫描 plugins 目录，dlopen 加载
- **运行**：调用注册函数，channel 开始接收消息，tools 可被调用
- **卸载**：停止 channel → 卸载 tools → dlclose

### 3.6 插件目录结构

```
~/.opencarrier/plugins/
  wecom/
    plugin.toml          # 插件元数据 + 配置
    libopencarrier_plugin_wecom.so   # 编译产物
    src/                 # 源码（如果源码包安装）
    data/                # 插件运行时数据（token 缓存等）
```

### 3.7 插件元数据（plugin.toml）

```toml
[plugin]
name = "wecom"
version = "1.0.0"
description = "企业微信集成"
author = "developer"
min_opencarrier_version = "0.1.0"

[channel]
type = "wecom"
# 消息通道类型标识

[[tools]]
name = "create_spreadsheet"
description = "创建企微表格"

[[tools]]
name = "add_rows"
description = "向企微表格添加行数据"

[[tools]]
name = "query_spreadsheet"
description = "查询企微表格数据"

[[tools]]
name = "send_app_message"
description = "发送企微应用消息"

[config_schema]
# 插件配置的 JSON Schema，用于 dashboard 渲染配置表单
# 每个 tenant 需要填写的字段
tenant_fields = [
  { name = "corp_id", type = "string", required = true },
  { name = "agent_id", type = "string", required = true },
  { name = "secret", type = "string", required = true, secret = true },
  { name = "token", type = "string", required = false },
  { name = "encoding_aes_key", type = "string", required = false, secret = true },
]

[[tenants]]
corp_id = "ww123456"
agent_id = "1000002"
secret = "..."  # 环境变量引用：env:WECOM_SECRET_CORP_A
```

## 4. 非功能需求

### 4.1 安全
- 插件二进制需签名验证（防止篡改）
- 源码包需 checksum 校验
- 插件运行在独立线程，崩溃不影响主程序
- 插件不能访问其他插件的内部状态
- secret 字段不落盘明文（支持 env: 引用）

### 4.2 性能
- 插件加载不阻塞主程序启动（异步加载）
- channel 消息处理和 tool 执行都是异步的
- token 缓存避免重复请求

### 4.3 可观测性
- 插件状态（loaded/running/error）可通过 API 查询
- 插件日志集成到主程序日志系统
- channel 和 tool 的调用指标（次数、延迟、错误率）

### 4.4 兼容性
- 插件声明 `min_opencarrier_version`，主程序拒绝加载不兼容版本
- plugin ABI 版本化，主程序和插件按 ABI 版本匹配

## 5. 约束

- 插件用 Rust 编写（cdylib ABI）
- 源码包方式要求用户机器有 Rust 工具链
- 插件与主程序共享同一个 tokio runtime
- 插件不能直接访问主程序的内存结构，只能通过 ABI 接口交互

## 6. 开放问题

1. **ABI 稳定性**：Rust 没有稳定 ABI，源码包方式可以解决，预编译二进制需要定义 C ABI 层
2. **插件间通信**：两个插件之间是否需要互相调用？（暂不需要）
3. **插件热更新**：运行中替换插件版本？（v2 再考虑）
4. **权限模型**：插件能访问哪些主程序能力？（网络？文件？只限自身目录？）
5. **Hub 审核流程**：插件上架是否需要人工审核？
