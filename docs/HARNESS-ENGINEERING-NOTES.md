# Harness Engineering 改进建议

> 基于 2026 年 AI 工程圈 Harness Engineering 趋势的改进建议
> 创建时间: 2026-03-22
> 状态: 待讨论（迁移完成后）

---

## 背景

2026 年 2 月起，**Harness Engineering** 成为 AI 工程圈最热话题。

核心观点：**模型不是关键，Harness 才是**。同一个模型，换一套运行环境，编程基准成功率从 42% 跳到 78%。

### 三代进化

| 阶段 | 时间 | 核心关注 |
|------|------|----------|
| Prompt Engineering | 2022-2024 | 精心构造单次指令 |
| Context Engineering | 2025 | 为每个决策点动态构建上下文 |
| Harness Engineering | 2026.2 起 | 设计完整的控制系统 |

### Harness 的定义

> Agent = AI Model + Harness

- **Scaffolding（脚手架）**: 系统 prompt 编译、工具 schema 构建、sub-agent 注册表
- **Harness（运行时编排）**: 核心推理循环的包装层，协调工具执行、上下文管理、安全执行
- **Context Engineering（上下文工程）**: Token 预算管理，决定什么信息该进来/压缩/丢弃

---

## OpenCarrier 现状分析

### 当前架构对应

| Harness 组件 | OpenCarrier 对应 |
|-------------|-----------------|
| 上下文管理 | `opencarrier-memory` (SQLite + vector) |
| 架构约束 | `opencarrier-kernel` (RBAC, 能力门控) |
| 工具链 | 53 工具 + MCP + A2A |
| 沙箱 | WASM 双计量沙箱 |
| 审计 | Merkle 哈希链审计 |
| 生命周期 | Hands 系统 |

### 路线定位

OpenCarrier 属于 **Big Harness 阵营**：
- 14 Rust crates
- 16 层安全系统
- 53 内置工具
- 40 渠道适配器
- 7 个 Hands

**对比**: Big Model 阵营（如 Claude Code）追求"最薄的那层包装"，让模型自己判断更多事情。

---

## 改进建议

### 1. 引入 AGENTS.md 文件

**问题**: 项目只有 `CLAUDE.md`（给 Claude CLI 用的），没有 `AGENTS.md`（给 OpenCarrier 内部 Agent 用的）。

**最佳实践** (ETH Zurich 研究):
- 控制在 60 行以内
- 写"目录"而非"百科全书"
- 每当 Agent 犯错，就加一条规则

**建议**:
- 在仓库根目录创建 `AGENTS.md`
- 每个 Hand 可能需要专门的指令文件
- 例如: `hands/researcher/RULES.md`, `hands/browser/RULES.md`

**Mitchell Hashimoto 的原则**:
> 每当你发现 Agent 犯了一个错误，你就花时间去工程化一个解决方案，让它再也不会犯同样的错。

---

### 2. 工具精简

**问题**: OpenCarrier 有 53 个工具，可能过多。

**案例** (Vercel):
- 从 15 个工具砍到 2 个
- 准确率从 80% 升到 100%

**案例** (Stripe):
- 有 500 个 MCP 工具
- 但每个 Agent 只能看到精心筛选过的子集

**建议**:
- 按 Hand 类型预设工具白名单
- 考虑动态加载机制
- "更多的工具并不等于更好的表现"

---

### 3. 反馈循环完善

**问题**: Agent 可能无法自己验证产出。

**OpenAI 的做法**:
- 每个 worktree 配本地可观测性栈
- Agent 可以自己查报错、看性能数据、定位问题
- 接了浏览器（Chrome DevTools Protocol），Agent 能看到 UI 渲染结果

**当前 OpenCarrier**:
- 有 `/api/metrics` 端点
- 但 Agent 可能没有工具去调用它

**建议**:
- 添加 `get_system_metrics` 工具
- 添加 `get_error_logs` 工具
- 让 Agent 能自己跑测试、查日志

---

### 4. 熵管理机制

**问题**: AI 生成的代码写多了，文档会过时，架构会漂移。

**OpenAI 的做法**:
- 定期启动专门的 Agent 扫描技术债
- 扫描文档不一致、架构违规等问题
- 提交修复 PR

**建议**:
- 新增 `entropy-manager` Hand
- 定期任务：扫描 TODO/FIXME、过时文档、架构漂移
- 类似"垃圾回收"的概念

---

### 5. CI 限速

**Stripe 的做法**:
- CI 最多跑两轮
- 第一轮失败 → Agent 自动修复 → 再跑一次
- 如果还失败 → 直接转交人类
- 不允许 Agent 在 CI 上无限重试

**建议**:
- 在 Hands 系统中加入类似限制
- 避免在错误方向上越跑越远

---

### 6. 确定性约束

**原则**: 约束比指令更有效。

**Cursor 的发现**:
> 告诉 Agent "不要留 TODO" 比告诉它 "完成所有实现" 效果更好。

**当前 OpenCarrier**:
- 16 层安全系统
- 但需要审视：这些是否真正阻止了 Agent 犯常见错误？

**建议**:
- 审计现有约束的实际效果
- 考虑添加更多"硬约束"（linter、类型检查、结构化测试）
- 把"老师傅的经验"写进约束系统

---

## 路线之争

### Big Model 阵营

**观点**: 模型能力的增长才是主旋律，Harness 只是权宜之计。

**代表**: OpenAI (Noam Brown), Claude Code

> "别花六个月搭建一个可能六个月后就被淘汰的东西。"

### Big Harness 阵营

**观点**: 模型是引擎，Harness 是方向盘和刹车。引擎再强，没有方向盘你也到不了目的地。

**代表**: Cursor, LangChain, OpenCarrier

> "Model Harness 就是一切。"

### 护栏悖论

> 车速越快，护栏越重要。

模型越强，越需要精心设计的约束系统确保它跑在正确的方向上。

**哪些会被淘汰**:
- 复杂的路由器
- 编排器
- multi-agent 协作框架

**哪些是持久价值**:
- 沙箱
- 审计链
- 架构约束
- 工具链
- 熵管理

---

## 行动建议

### 短期 (迁移完成后)

1. 创建 `AGENTS.md` 文件（60 行以内）
2. 为 7 个 Hands 各创建 `RULES.md`
3. 审计工具使用频率，考虑精简

### 中期

1. 添加熵管理机制（定期扫描技术债）
2. 完善反馈循环（让 Agent 能自己查日志、跑测试）
3. 引入 CI 限速机制

### 长期

1. 思考 Big Harness 路线的可持续性
2. 识别哪些组件会被更强的模型"吃掉"
3. 保持 Harness 轻量化、模块化

> "Start Simple. Build to Delete." — Philipp Schmid

---

## 参考资料

- [OpenAI 博文: Harness Engineering](https://openai.com/index/harness-engineering/)
- [Mitchell Hashimoto: My AI Adoption Journey](https://mitchellh.com/writing/my-ai-adoption-journey)
- [Martin Fowler: Harness Engineering](https://martinfowler.com/articles/exploring-gen-ai/harness-engineering.html)
- [Philipp Schmid: Agent Harness 2026](https://www.philschmid.de/agent-harness-2026)
- [Latent Space: Is Harness Engineering Real?](https://www.latent.space/p/ainews-is-harness-engineering-real)
- [Stripe: Minions - One-shot End-to-End Coding Agents](https://stripe.dev/blog/minions-stripes-one-shot-end-to-end-coding-agents)
- [Cursor: Self-Driving Codebases](https://cursor.com/blog/self-driving-codebases)
- [arXiv: Building Effective AI Coding Agents](https://arxiv.org/abs/2603.05344v3)

---

**最后更新**: 2026-03-22
**状态**: 待讨论
