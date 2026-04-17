---
name: training-guide
source: manual
type: knowledge
description: 分身训练五维度方法论 — 知识、技能、人格、行为、配置的详细操作指南
tags: ["training", "methodology", "guide"]
confidence: EXTRACTED
status: active
---

# 分身训练方法论

分身训练是一个通过 LLM 介入的深度优化过程，不是简单的文件读写。训练涵盖五个维度，每个维度有不同的处理流程和目标文件。

## 维度 1: 知识喂养

### 核心原则
知识喂入必须经过 LLM 处理，不能把原始内容直接灌入。LLM 的职责：
- 提取关键事实和概念
- 去除冗余和重复
- 按主题分类整理
- 生成结构化的知识条目

### 操作流程
1. 接收原始资料（用户粘贴、文件、URL）
2. 分析资料类型（FAQ、聊天记录、技术文档、产品说明等）
3. 使用 `knowledge_import`（自动解析）或手动用 `knowledge_add`（LLM 处理后写入）
4. 运行 `knowledge_lint` 检查质量
5. 必要时用 `knowledge_heal` 修复问题

### 知识文件格式
```markdown
---
name: <标题>
source: manual
type: knowledge
description: <一句话描述>
tags: [<tag1>, <tag2>]
confidence: EXTRACTED|INFERRED|AMBIGUOUS
status: active
---

<结构化知识内容>

---

- <YYYY-MM-DD>: <来源说明>
```

### 质量标准
- 每个文件聚焦一个主题，1000-3000 字
- 必须有完整的 YAML frontmatter
- confidence 标签反映知识来源的可靠性
- description 准确概括内容

## 维度 2: 技能训练

### 技能设计要点
- `when_to_use` 必须明确具体，不能是"用户需要帮助"这种模糊描述
- `allowed_tools` 只包含执行该技能真正需要的工具
- 步骤描述要清晰、可操作、无歧义

### 技能文件格式
```yaml
---
name: <skill-name>
when_to_use: <具体的触发条件>
allowed_tools: ["<tool1>", "<tool2>"]
version: 1
usage_count: 0
---
# <Skill Name>
<执行步骤>
```

### 设计原则
- 一个技能解决一类问题
- 步骤不超过 8 步
- 工具集最小化，避免工具冗余

## 维度 3: 人格塑造

### SOUL.md 设计原则
- 使用自然语言描述，不使用机械的列表
- 包含：性格、语气、情感模式、行为边界
- 不包含：工作规则、流程、知识事实
- 保持人格一致性，修改时考虑与现有描述的兼容

### 配套修改
修改人格后，检查 `system_prompt.md` 是否需要同步调整：
- 沟通风格变化 → 修改输出格式部分
- 专业领域变化 → 修改核心能力部分
- 角色定位变化 → 修改角色描述部分

## 维度 4: 行为调整

### 常见行为调整需求
- 输出太长/太短 → 调整 system_prompt 中的输出格式要求
- 回答不准确 → 补充领域知识到知识库
- 不遵守规则 → 加强 system_prompt 中的约束条件
- 工具使用不当 → 调整工具配置或补充工具使用说明

### 修改策略
- 在 system_prompt.md 中找到对应部分修改
- 不删除现有的有效规则，只添加或调整
- 复杂的行为调整可能同时涉及知识库和技能

## 维度 5: 配置调整

### agent.toml 常见调整
- 模型切换：provider + model 字段
- 温度调整：temperature（精确任务用 0.3，创意任务用 0.7-1.0）
- 工具增减：capabilities.tools 列表
- 资源限制：resources 下的各项限制
- 自动审批：exec_policy.auto_approve

### 注意事项
- 修改 agent.toml 后需要重启分身才能生效
- 工具变更影响分身的能力边界，需谨慎
- 不要同时修改太多配置项

## 训练效果验证

每次训练后应执行 `clone_evaluate` 评估质量：
- 评分 >= 80：优秀
- 评分 >= 60：良好
- 评分 >= 40：及格
- 评分 < 40：需改进

关注评分变化趋势，确保训练有效。

---

- 2026-04-17: 从训练系统设计文档手动创建
