# Claude Code v2.1.88 原版源码架构分析

> 来源: `/Users/sophiehe/Downloads/claude-code-source-code-main/`
> 分析日期: 2026-04-02
> 代码规模: 1884 文件, 51 万行 TypeScript
> 用途: 仅提取设计思路，供 OpenCarrier (Rust) 参考

---

## 一、整体架构概览

```
src/
├── assistant/       # Agent 核心循环
├── coordinator/     # 多 Agent 任务协调 (Leader-Worker 模式)
├── services/        # 基础设施层 (API, MCP, OAuth, 插件, 工具执行)
├── tools/           # 30+ 工具定义和实现
├── skills/          # 技能系统 (bundled + 磁盘 + MCP + 插件)
├── plugins/         # 插件框架
├── hooks/           # Hook 系统 (25+ 事件类型)
├── commands/        # 70+ 斜杠命令
├── context/         # 上下文管理
├── state/           # 状态管理 (类 Zustand)
├── query/           # QueryEngine + 令牌预算
├── memdir/          # 基于文件的持久记忆系统
├── cli/             # CLI 入口和传输层
├── server/          # HTTP 服务
├── bridge/          # REPL 桥接层
├── vim/             # Vim 编辑器
├── ink/             # Ink React 终端 UI
├── buddy/           # 吉祥物系统 (非功能性)
├── voice/           # 语音模式
└── bootstrap/       # 启动引导
```

---

## 二、工具系统 (对 OpenCarrier 最有参考价值)

### 工具定义模式

每个工具是一个对象，包含:
- `name` — 唯一标识符
- `inputSchema` — Zod schema 验证模型输入
- `call(args, context, canUseTool, parentMessage, onProgress)` — 执行函数
- `description(input, options)` — 动态描述 (发送给 API)
- `prompt(options)` — 工具的系统 prompt 部分
- `mapToolResultToToolResultBlockParam()` — 输出序列化回 API 格式

可选生命周期: `validateInput`, `checkPermissions`, `isConcurrencySafe`, `isReadOnly`, `isDestructive`, `interruptBehavior`

**关键设计**: `buildTool()` 用 fail-closed 默认值 — `isConcurrencySafe` 默认 false, `isReadOnly` 默认 false。工具必须显式声明危险能力。

### 工具注册

`tools.ts` 是中央注册表:
1. `getAllBaseTools()` 返回所有内置工具数组
2. 条件包含: 功能门控 (`feature('PROACTIVE')`, `feature('KAIROS')`) 或环境变量
3. `getTools(permissionContext)` 过滤: deny 规则 → REPL 模式隐藏 → 禁用检查
4. `assembleToolPool()` 合并内置 + MCP 工具，去重 (内置优先)，按名称排序

**Prompt Cache 稳定性**: 工具按名称排序，内置工具作为连续前缀放在 MCP 工具之前。添加/移除 MCP 工具不会使内置工具部分的 prompt cache 失效。

### 工具执行管道 (5 层权限)

```
Zod schema 验证 → validateInput() → backfillObservableInput()
    → Pre-tool Hooks (allow/deny/ask/modify)
        → 规则权限 (settings.json deny/ask)
            → 交互式 canUseTool()
                → tool.call()
                    → Post-tool Hooks
```

**关键**: Hook allow 不绕过 deny 规则 — 纵深防御。

### 流式工具执行

`StreamingToolExecutor` 允许工具在 API 响应流式到达时就开始执行:
- 按工具的 `isConcurrencySafe` 分批: 安全工具并行，不安全工具串行
- 跟踪每个工具状态: `queued → executing → completed → yielded`
- Bash 工具错误时立即杀死兄弟子进程

### 工具搜索 / 延迟加载

工具池超过阈值时标记 `defer_loading: true`。`ToolSearchTool` 作为元工具:
- 模型先调用 ToolSearch 发现可用工具
- 再按需加载，减少初始 prompt 大小
- 模糊匹配 + 评分系统

### 异步生成器管道

整个执行模型基于 `AsyncGenerator`:
- API 流式 → 工具执行 → 结果发射，全部是 async iterable
- 支持 backpressure 和 `AbortController` 取消
- 进度报告穿插在生成器 yield 中

---

## 三、Hook 系统 (25+ 事件)

### Hook 事件类型

| 类别 | 事件 |
|------|------|
| 工具生命周期 | `PreToolUse`, `PostToolUse`, `PostToolUseFailure`, `PermissionDenied`, `PermissionRequest` |
| 会话生命周期 | `SessionStart`, `SessionEnd`, `Setup`, `ConfigChange`, `InstructionsLoaded` |
| Agent 生命周期 | `SubagentStart`, `SubagentStop`, `Stop`, `StopFailure` |
| 压缩 | `PreCompact`, `PostCompact` |
| 用户交互 | `UserPromptSubmit`, `Notification`, `Elicitation`, `ElicitationResult` |
| 工作区 | `CwdChanged`, `FileChanged`, `WorktreeCreate`, `WorktreeRemove` |
| 团队 | `TeammateIdle`, `TaskCreated`, `TaskCompleted` |

### Hook 命令类型 (判别联合)

- `command` — shell 命令 (bash/powershell)
- `prompt` — LLM prompt 评估
- `agent` — 多轮 Agent 验证器
- `http` — HTTP POST 请求

所有类型支持: `if` (条件规则), `once` (一次性), `async` (非阻塞), `statusMessage`

### Hook 来源 (分层)

`managed > user > project > local > policy > plugin > session > builtin`

### Hook 执行

- Shell 命令: `spawn()` 运行, JSON 输入通过 stdin
- 退出码: `0` = 成功, `2` = 阻止/注入, 其他 = 警告
- Hook 可返回 JSON 修改行为: 阻止工具调用、注入消息、更新权限

---

## 四、技能系统

### 四种加载机制

1. **Bundled Skills** — 编译时注册到数组，约 17 个内置技能
2. **磁盘 Skills** — `skills/name/SKILL.md` 文件，YAML frontmatter 配置
3. **MCP Skills** — MCP 服务器暴露的技能，MCP 技能不执行内联 shell 命令 (安全约束)
4. **插件 Skills** — 插件目录下的技能，标记 `loadedFrom: 'plugin'`

### Skill Frontmatter 字段

`description`, `allowed-tools`, `model`, `effort`, `hooks`, `context` (inline|fork), `agent`, `paths`, `user-invocable`, `shell`

### 发现层级

`managed > user > project > 附加目录`

- 条件技能: `paths:` frontmatter 休眠直到匹配
- 动态发现: 工具操作触及 `.claude/skills/` 嵌套目录时即时发现
- `realpath()` 去重，`memoize` 缓存

---

## 五、插件系统

### 插件类型

| 类型 | ID 格式 | 特点 |
|------|---------|------|
| 内置 | `{name}@builtin` | TypeScript 注册，可提供技能/钩子/MCP |
| 市场 | `{name}@marketplace` | Git/npm 下载，完整生命周期管理 |

### 插件目录结构

```
my-plugin/
├── plugin.json        # 清单
├── commands/           # 自定义斜杠命令 (.md)
├── agents/             # 自定义 Agent (.md)
├── skills/             # 技能目录
└── hooks/hooks.json    # 钩子定义
```

### 命名空间

插件命令用 `pluginName:commandName` 防止冲突。

### 插件变量替换

`${CLAUDE_PLUGIN_ROOT}`, `${CLAUDE_PLUGIN_DATA}`, `${CLAUDE_SESSION_ID}`, `${user_config.X}`

---

## 六、QueryEngine 和令牌预算

### QueryEngine (每会话一个实例)

每回合生命周期:
1. `processUserInput` — 处理斜杠命令、附件
2. 用户消息持久化到日志 (崩溃可恢复)
3. `fetchSystemPromptParts` — 构建系统 prompt (工具、MCP、Agent 定义)
4. 调用 `query()` 异步生成器
5. 逐步 yield 标准化 SDK 消息
6. 累积 usage
7. 检查预算限制 (`maxBudgetUsd`) 和最大回合数
8. 结束时刷新日志、计算结果

### 依赖注入

4 个可注入依赖: `callModel`, `microcompact`, `autocompact`, `uuid`
测试注入 mock，生产用 `productionDeps()`。

### 令牌预算 — "推测"继续模式

- 对话达到上下文窗口 90% 时发送渐进 prompt
- 检测收益递减: 3 次以上连续尝试每次 <500 token → 停止自动继续

### 压缩策略

- `autoCompactIfNeeded` — 自动触发
- `microCompact` — 增量编辑压缩
- `HISTORY_SNIP` — "裁剪压缩"，旧消息替换为短摘要

---

## 七、记忆系统 (memdir)

### 核心设计

基于文件的持久记忆，每项目一个目录: `~/.claude/projects/<sanitized-git-root>/memory/`
Git worktree 共享规范根目录 → 共享记忆目录。

### MEMORY.md (入口文件)

最多 200 行或 25KB。每行是指向主题文件的指针。主题文件有 `name`, `description`, `type` frontmatter。

### 四种记忆类型

| 类型 | 用途 |
|------|------|
| `user` | 用户角色、偏好、专业知识 (始终私有) |
| `feedback` | 行为指导 ("做这个"/"不要那个") |
| `project` | 进行中的工作、目标、截止日期 |
| `reference` | 指向外部系统的指针 (Linear board, Grafana) |

明确排除: 代码模式、架构、Git 历史 — 可从代码库派生的不存。

### 记忆召回

查询时扫描最多 200 个 .md 文件 (按最新修改排序)。Sonnet 模型收到文件名+描述清单，选择最多 5 个相关记忆。使用 `sideQuery()` — 旁路 LLM 调用。

### 记忆新鲜度

大于 1 天的记忆附加警告: "记忆是时间点观察，不是实时状态"。防止模型依赖过时路径/函数名。

### 团队记忆

`memory/team/` 子目录，支持共享范围 (所有贡献者可见) vs 私有范围 (仅用户)。
严格安全验证防止 symlink 路径遍历攻击。

---

## 八、多 Agent 协调器 (Coordinator Mode)

### 激活

`CLAUDE_CODE_COORDINATOR_MODE=1` 环境变量 + `COORDINATOR_MODE` 功能门控。

### 两层架构

- **Coordinator (Leader)** — 与用户通信，分解任务为 研究/实施/验证 阶段，生成 Worker Agent。自身不执行代码。
- **Worker Agent** — 自主 Agent，可访问 Bash/Read/Edit (简单模式) 或全套工具+MCP+技能 (完整模式)。

### 关键原则

1. **隔离**: Worker 看不到 Coordinator 的对话。Worker prompt 必须自包含。
2. **综合**: Coordinator 必须阅读 Worker 发现，理解后合成具体规范。反模式: "根据你的发现修复 bug"。
3. **并行**: 研究 Worker 并发启动。实施按文件集序列化。验证用新 Worker。
4. **继续 vs 生成**: 高上下文重叠时继续已有 Worker，低重叠时生成新 Worker。

### 结果传递

作为 `<task-notification>` XML 块在 user-role 消息中传递，包含 `task-id`, `status`, `summary`, `result`, `usage`。

### 草稿本

`tengu_scratch` 功能门控激活时，Coordinator 和 Worker 共享草稿本目录，用于持久跨 Worker 知识，无需权限提示即可读写。

---

## 九、状态管理

### 存储原语

极简 Zustand 风格: `getState()`, `setState(updater)`, `subscribe(listener)`。
`setState` 用 `Object.is` 标识检查，未变化则跳过通知。仅 34 行代码。

### AppStateStore

570+ 行类型定义，`DeepImmutable` 包装 (函数类型字段除外)。关键领域:
- 设置和模型配置
- 工具权限上下文和权限模式
- 任务跟踪 (按任务 ID) 和 Agent 名称注册表
- 团队上下文 (队友协调)
- MCP 客户端/工具/命令/资源
- 插件系统 (启用/禁用/错误/安装状态)
- 文件历史跟踪和提交归属

### React 集成

`useAppState(selector)` 使用 `useSyncExternalStore`，只有选中切片变化才重渲染。
`useSetAppState()` 返回稳定 setter，从不导致重渲染。

### onChangeAppState

状态变更的单点拦截:
- 权限模式变更 → 同步到 CCR/SDK
- 模型/详细度/expandedView 变更 → 持久化到全局配置
- 设置变更 → 清除认证缓存

---

## 十、服务层

### API 客户端

`getAnthropicClient()` 工厂按提供商创建 SDK 客户端:
- 第一方 Anthropic (API key)
- AWS Bedrock (AWS credentials)
- Azure Foundry (Azure AD)
- Google Vertex AI (Google ADC)

`claude.ts` (~3400 行) 核心交互层: 流式/非流式查询、工具搜索、prompt cache 断点、beta header 管理、消息规范化、详细 usage/cost 跟踪。

### MCP 服务

- 传输: Stdio, SSE, HTTP, WebSocket, SDK, claudeai-proxy
- 连接状态: `ConnectedMCPServer | FailedMCPServer | NeedsAuthMCPServer | PendingMCPServer | DisabledMCPServer`
- 工具命名: `mcp__serverName__toolName`
- React context 管理重连和切换

### 压缩服务

`compact.ts` (主压缩), `microCompact.ts` (增量), `autoCompact.ts` (自动触发), `sessionMemoryCompact.ts` (会话级记忆压缩)

### VCR 测试

`withVCR`/`withStreamingVCR` 录制/回放 API 交互，确定性测试不需要真实 API。

---

## 十一、成本跟踪

跟踪: 总美元成本、API 持续时间、工具持续时间、代码行变更、按模型 usage (input/output/cache read/cache write)、Web 搜索请求数。

成本计算: `calculateUSDCost(model, usage)` — 顾问模型用量递归计算并叠加到会话成本。

会话持久化: `saveCurrentSessionCosts` 写入项目配置，`restoreCostStateForSession` 仅在 session ID 匹配时恢复。

---

## 十二、关键设计模式总结 (OpenCarrier 可借鉴)

### 高优先级

1. **工具 Fail-Closed 默认值** — `isConcurrencySafe=false`, `isReadOnly=false`，必须显式声明危险能力
2. **5 层权限纵深防御** — Schema 验证 → validateInput → Hooks → 规则权限 → 交互式确认
3. **流式工具执行** — 工具在 API 流式到达时就开始执行，按并发安全分批
4. **工具延迟加载** — 工具池过大时用 ToolSearch 元工具按需发现
5. **Hook 多命令类型** — command / prompt / agent / http，不仅是 shell 命令
6. **多 Agent Leader-Worker 协调** — Coordinator 不执行代码，只分解和综合
7. **Prompt Cache 稳定性** — 内置工具连续前缀，MCP 工具在后，排序固定

### 中优先级

8. **基于文件的持久记忆** — MEMORY.md 索引 + 主题文件，旁路 LLM 召回
9. **记忆新鲜度警告** — >1天的记忆标注"非实时"
10. **推测继续模式** — 90% 窗口时渐进 prompt，检测收益递减自动停止
11. **AsyncGenerator 管道** — 整个执行模型基于 async iterable
12. **依赖注入** — QueryEngine 的 4 个可替换依赖
13. **VCR 测试** — 录制/回放 API 交互

### 低优先级

14. **判别联合类型** — Command, HookCommand 等用 discriminated union
15. **分层设置源** — managed > user > project > local > policy > plugin > session > builtin
16. **插件命名空间** — `pluginName:commandName` 防冲突
17. **动态技能发现** — 工具操作触及嵌套 .claude/skills/ 时即时发现
18. **onChange 单点拦截** — 状态变更的副作用集中管理
19. **草稿本** — 跨 Agent 共享文件目录用于持久知识交换
