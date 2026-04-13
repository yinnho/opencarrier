# Claude 插件转换指南

> 将 Claude Code 官方插件转换为 opencarrier 分身（.agx）
> 最后更新: 2026-04-12 | 适用: clone-creator v3+

---

## 1. 前置条件

```bash
# opencarrier 已运行
opencarrier start

# clone-creator v3 已安装（含 3 个子代理）
opencarrier agent list   # 应显示 clone-creator

# CLI 支持 -m 非交互模式
opencarrier agent chat --help   # 应显示 -m, --message 选项
```

---

## 2. 转换流程

### Step 1: 用 `-f` 导入插件文件，发送转换请求

```bash
# 导入插件的 SKILL.md 和 plugin.json，附带转换指令
opencarrier agent chat clone-creator \
  -f ~/Downloads/claude-plugins-official-main/plugins/frontend-design/skills/frontend-design/SKILL.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/frontend-design/.claude-plugin/plugin.json \
  -m '上面是一个 Claude Code 插件的文件。请把它转换为一个 opencarrier 分身，名字用 frontend-design。不需要子代理，进化策略用 conservative。'
```

**`-f` 参数说明**：
- 可以指定多个 `-f`，每个导入一个文件
- `-f -` 表示从 stdin 读取（支持管道）
- 文件内容会附带文件名前缀拼接到消息前面
- 配合 `-m` 附加转换指令

**对于 Command+Agents 类插件**（如 feature-dev）：

```bash
opencarrier agent chat clone-creator \
  -f ~/Downloads/claude-plugins-official-main/plugins/feature-dev/commands/feature-dev.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/feature-dev/agents/code-explorer.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/feature-dev/agents/code-architect.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/feature-dev/agents/code-reviewer.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/feature-dev/.claude-plugin/plugin.json \
  -m '上面是 Claude Code 的 feature-dev 插件。请转换为 opencarrier 分身，名字用 feature-dev。agents 可以直接映射。'
```

**用管道批量导入**：

```bash
# 把整个插件目录所有 md/json 文件拼起来导入
cat ~/Downloads/claude-plugins-official-main/plugins/code-review/commands/*.md \
    ~/Downloads/claude-plugins-official-main/plugins/code-review/.claude-plugin/plugin.json \
  | opencarrier agent chat clone-creator \
    -f - \
    -m '这是 code-review 插件。请转换为分身，名字用 code-review。'
```

### Step 2: clone-creator 自动工作

clone-creator 读取插件文件后会自动：

1. 分析插件内容 — 理解功能、定位、agents 结构
2. 生成 SOUL.md — 从插件描述推断人格
3. 生成 system_prompt.md — 将插件指令转换为分身行为规则
4. 拆分知识文件 — 将插件中的指南按主题拆到 knowledge/
5. 映射 agents — 将插件的 `agents/*.md` 直接转换为分身的 `agents/`
6. 设计技能 — 生成 skills/
7. 生成 EVOLUTION.md — 配置进化策略
8. 输出所有文件到 `output/` 目录

### Step 3: 打包安装

clone-creator 会输出一个 `install-*.sh` 脚本到 output/，或者手动打包：

```bash
# clone-creator 生成的文件在 output/ 目录
# 文件名格式：<name>-<type>.md 或 <name>-knowledge-<topic>.md
OUTPUT=~/.opencarrier/workspaces/clone-creator/output

# 手动组装（或使用 clone-creator 生成的 install 脚本）
mkdir -p /tmp/<name>/{knowledge,skills,agents}
cp $OUTPUT/<prefix>-template.json /tmp/<name>/template.json
cp $OUTPUT/<prefix>-profile.md /tmp/<name>/profile.md
cp $OUTPUT/<prefix>-SOUL.md /tmp/<name>/SOUL.md
cp $OUTPUT/<prefix>-system_prompt.md /tmp/<name>/system_prompt.md
cp $OUTPUT/<prefix>-MEMORY.md /tmp/<name>/MEMORY.md
cp $OUTPUT/<prefix>-EVOLUTION.md /tmp/<name>/EVOLUTION.md
cp $OUTPUT/<prefix>-knowledge-*.md /tmp/<name>/knowledge/
cp $OUTPUT/<prefix>-skills-*.md /tmp/<name>/skills/
# 如果有 agents
cp $OUTPUT/<prefix>-agents-*.md /tmp/<name>/agents/

# 打包
cd /tmp && tar czf <name>.agx -C <name> .

# 安装
BASE64=$(base64 -w0 /tmp/<name>.agx)
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64\"}"
```

### Step 4: 验证

```bash
# 确认安装
opencarrier agent list

# 测试使用
opencarrier agent chat frontend-design -m "帮我设计一个音乐 App 的仪表盘"
```

---

## 3. 实战案例：frontend-design

### 输入：通过 `-f` 导入插件原始文件

```bash
opencarrier agent chat clone-creator \
  -f ~/Downloads/claude-plugins-official-main/plugins/frontend-design/skills/frontend-design/SKILL.md \
  -f ~/Downloads/claude-plugins-official-main/plugins/frontend-design/.claude-plugin/plugin.json \
  -m '上面是一个 Claude Code 插件的文件。请把它转换为一个 opencarrier 分身，名字用 frontend-design-v2。不需要子代理，进化策略用 conservative。'
```

**关键**：`-f` 把插件的 SKILL.md 和 plugin.json 原文直接喂给 clone-creator，而不是手动描述。这样 clone-creator 能看到完整的指令内容和格式。

### clone-creator 生成的文件（13 个，费用 $0.08，9 轮）

| 文件 | 大小 | 来源 |
|------|------|------|
| `template.json` | 329B | plugin.json 转换 |
| `profile.md` | 488B | plugin.json 转换 |
| `SOUL.md` | 1.7KB | 从插件描述推断 |
| `system_prompt.md` | 4.4KB | SKILL.md 的设计思维+行为约束 |
| `MEMORY.md` | 448B | 自动生成 |
| `EVOLUTION.md` | 727B | conservative |
| `knowledge/typography-guide.md` | 2.8KB | SKILL.md Typography 段 |
| `knowledge/color-palette-guide.md` | 2.9KB | SKILL.md Color & Theme 段 |
| `knowledge/animation-and-motion.md` | 3.2KB | SKILL.md Motion 段 |
| `knowledge/spatial-composition.md` | 3.0KB | SKILL.md Spatial Composition 段 |
| `knowledge/anti-ai-aesthetic-patterns.md` | 3.8KB | SKILL.md NEVER 段 |
| `skills/design-and-implement.md` | 1.9KB | 从 SKILL.md 工作流提取 |
| `install-fdv2.sh` | 2.3KB | 自动生成的安装脚本 |

### 原始插件到分身的映射关系（clone-creator 自动生成）

| 原始插件内容 | → 分身中的位置 |
|---|---|
| SKILL.md → Design Thinking | → `system_prompt.md` Phase 1-2 |
| SKILL.md → Typography | → `knowledge/typography-guide.md` |
| SKILL.md → Color & Theme | → `knowledge/color-palette-guide.md` |
| SKILL.md → Motion | → `knowledge/animation-and-motion.md` |
| SKILL.md → Spatial Composition | → `knowledge/spatial-composition.md` |
| SKILL.md → Backgrounds & Visual Details | → `system_prompt.md` 美学工程第 5 点 |
| SKILL.md → NEVER use generic AI aesthetics | → `knowledge/anti-ai-aesthetic-patterns.md` |
| plugin.json → name/description | → `template.json` + `profile.md` |

### 测试验证

```bash
# 安装
BASE64=$(base64 -w0 /tmp/frontend-design.agx)
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" -d "{\"data\": \"$BASE64\"}"

# 测试
opencarrier agent chat frontend-design -m "帮我设计一个音乐流媒体 App 的仪表盘界面"
```

测试结果 — frontend-design 选择了"暗夜录音室"设计方向：
- 字体：Syne + DM Sans + JetBrains Mono（避开了 Inter/Roboto）
- 配色：`#0d0f14` 深蓝黑底 + `#e8a849` 琥珀金强调（避开了紫色渐变）
- 布局：左侧导航 + 网格主区 + 右侧播放面板
- 动效：卡片 hover 微光 + 播放按钮呼吸脉冲 + 交错淡入

---

## 4. Claude Plugin → OpenCarrier Clone 映射规则

### 文件映射

| Claude Plugin | → OpenCarrier Clone | 说明 |
|---|---|---|
| `plugin.json` | `template.json` + `profile.md` | 元数据直译 |
| `skills/*/SKILL.md` | `system_prompt.md` | 核心指令成为行为规则 |
| `commands/*.md` | `system_prompt.md` + `skills/*.md` | 工作流进 prompt，子步骤进 skills |
| `agents/*.md` | `agents/*.md` | **直接映射**（几乎 1:1） |
| 无 | `SOUL.md` | 从插件描述推断人格 |
| 无 | `EVOLUTION.md` | 生成进化策略 |
| 无 | `knowledge/*.md` | 从插件内容按主题拆分 |
| `references/*.md` | `knowledge/*.md` | 参考文档直接映射 |

### Agent 字段映射

| Claude Agent | → OpenCarrier Agent |
|---|---|
| `name` | `name` |
| `description` | `description` |
| `tools` | `tools` |
| `model: sonnet/haiku` | `model: sonnet/haiku` |
| `color` | `color` |

---

## 5. 按插件类型的转换策略

### Skill 类（最简单）

frontend-design、explanatory-output-style、learning-output-style、code-simplifier

- SKILL.md 内容 → system_prompt.md
- 设计原则和反模式 → 拆分为 knowledge/ 文件
- 生成 conservative EVOLUTION.md

### Command 类（中等）

code-review、commit-commands、playground、ralph-loop、math-olympiad

- Command 流程 → system_prompt.md
- 内联 agent 描述 → agents/*.md
- 示例和规则 → knowledge/*.md
- 工具白名单 → agent 的 tools 字段

### Command+Agents 类（最佳映射）

feature-dev、pr-review-toolkit、plugin-dev、mcp-server-dev

- Command → system_prompt.md
- agents/*.md → agents/*.md（**直接映射，1:1**）
- 这正是 agents/ 目录最有价值的场景

### 不适合转换的

| 类型 | 原因 | 例子 |
|---|---|---|
| LSP 类 | IDE 语言服务器配置 | rust-analyzer, typescript, gopls 等 |
| Hook 类 | 工具拦截，opencarrier 无此机制 | security-guidance |
| Setup 类 | 环境配置工具 | claude-code-setup |

---

## 6. 32 个官方插件转换评估

| 插件名 | 类型 | 可转换 | 难度 |
|---|---|---|---|
| frontend-design | Skill | ✅ 已完成 | 低 |
| explanatory-output-style | Skill | ✅ | 低 |
| learning-output-style | Skill | ✅ | 低 |
| code-simplifier | Skill | ✅ | 低 |
| code-review | Command | ✅ | 中 |
| commit-commands | Command | ✅ | 低 |
| playground | Command | ✅ | 低 |
| ralph-loop | Command | ✅ | 低 |
| math-olympiad | Command | ✅ | 低 |
| hookify | Command | ✅ | 低 |
| claude-md-management | Command | ✅ | 低 |
| agent-sdk-dev | Command | ✅ | 中 |
| feature-dev | Command+Agents | ✅ | 低（1:1映射）|
| pr-review-toolkit | Command+Agents | ✅ | 中 |
| plugin-dev | Command+Agents | ✅ | 中 |
| mcp-server-dev | Command+Agents | ✅ | 中 |
| skill-creator | Skill+Agents+Scripts | ✅ | 高 |
| security-guidance | Hook | ❌ | — |
| claude-code-setup | Setup | ❌ | — |
| rust-analyzer-lsp | LSP | ❌ | — |
| typescript-lsp | LSP | ❌ | — |
| gopls-lsp | LSP | ❌ | — |
| pyright-lsp | LSP | ❌ | — |
| clangd-lsp | LSP | ❌ | — |
| kotlin-lsp | LSP | ❌ | — |
| lua-lsp | LSP | ❌ | — |
| jdtls-lsp | LSP | ❌ | — |
| php-lsp | LSP | ❌ | — |
| ruby-lsp | LSP | ❌ | — |
| swift-lsp | LSP | ❌ | — |
| csharp-lsp | LSP | ❌ | — |
| example-plugin | Demo | ❌ | — |

**总计**: 17 个可转换（1 个已完成），15 个不适合。

---

## 7. CLI 命令参考

```bash
# 非交互式发消息（单次，打印回复后退出）
opencarrier agent chat <name-or-id> -m "消息内容"

# 带文件导入的消息（-f 可多次使用）
opencarrier agent chat clone-creator \
  -f plugin/SKILL.md \
  -f plugin/plugin.json \
  -m "请把这个插件转换为分身"

# 从管道读取文件
cat plugin/*.md | opencarrier agent chat clone-creator -f - -m "转换为分身"

# 交互式聊天（进入 TUI）
opencarrier agent chat <name-or-id>

# 列出所有分身
opencarrier agent list

# 安装 .agx 分身
BASE64=$(base64 -w0 <name>.agx)
curl -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64\"}"

# 停止分身
curl -X POST http://localhost:4200/api/clones/<name>/stop

# 卸载分身
curl -X DELETE http://localhost:4200/api/clones/<name>
```
