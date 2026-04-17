---
name: train-clone
when_to_use: 用户想训练或优化某个已有的分身，包括喂知识、训技能、塑人格、调行为、改配置等需求
allowed_tools: ["file_read", "file_write", "file_list", "knowledge_add", "knowledge_import", "knowledge_lint", "knowledge_heal", "knowledge_list", "knowledge_read", "knowledge_remove", "clone_evaluate", "memory_store", "memory_recall"]
version: 1
usage_count: 0
---

# 分身训练技能

当用户表达了训练或优化某个分身的意图时，执行以下流程。

## 流程

### 1. 意图识别

从用户消息中提取：
- **目标分身**：要训练哪个分身（名称或描述）
- **训练维度**：喂知识 / 训技能 / 塑人格 / 调行为 / 改配置
- **具体需求**：要达成什么效果

如果维度不明确，根据描述自动判断：
- 提到"知识"、"资料"、"文档"、"FAQ" → 喂知识
- 提到"技能"、"能力"、"功能" → 训技能
- 提到"性格"、"语气"、"风格"、"人设" → 塑人格
- 提到"行为"、"流程"、"格式"、"约束" → 调行为
- 提到"模型"、"温度"、"工具"、"配置" → 改配置

### 2. 读取现状

读取目标分身的工作区文件（路径：`~/.opencarrier/workspaces/{分身名}/`）：
- `agent.toml` — 配置和工具
- `SOUL.md` — 人格定义
- `system_prompt.md` — 行为指令
- `skills/` — 已有技能列表
- 使用 `knowledge_list` 查看已有知识
- 使用 `clone_evaluate` 获取当前质量评分（作为训练前基线）

### 3. 执行训练

根据维度执行对应操作：

#### 喂知识
1. 接收并分析原始资料
2. 用 LLM 能力提取结构化知识
3. 使用 `knowledge_add` 或 `knowledge_import` 写入
4. 运行 `knowledge_lint` 检查质量
5. 如有问题用 `knowledge_heal` 修复

#### 训技能
1. 分析分身定位和缺失能力
2. 设计 `when_to_use`（必须具体明确）
3. 定义最小工具集 `allowed_tools`
4. 编写清晰执行步骤
5. 用 `file_write` 写入 `skills/` 目录

#### 塑人格
1. 读取当前 `SOUL.md`
2. 根据用户需求调整人格描述
3. 用 `file_write` 更新 `SOUL.md`
4. 检查 `system_prompt.md` 是否需要配套修改

#### 调行为
1. 收集并分析行为反馈
2. 将反馈转化为行为规则
3. 修改 `system_prompt.md` 的对应部分
4. 必要时补充知识到知识库

#### 改配置
1. 读取 `agent.toml`
2. 精确修改目标字段
3. 用 `file_write` 写回
4. 提醒用户需要重启分身

### 4. 验证效果

1. 运行 `clone_evaluate` 获取训练后评分
2. 对比基线评分，确认训练有效
3. 如评分下降，分析原因并修复

### 5. 汇报结果

告诉用户：
- 训练了哪个维度
- 修改了哪些文件
- 质量评分变化：{训练前} → {训练后}
- 是否需要重启分身

## 重要原则

- 先读取现状再动手，不要盲改
- 每次只改用户要求的部分
- 知识喂入必须经过 LLM 处理，不灌原始内容
- 保持知识文件双层格式
- 保持 TOML 格式正确
- 训练后必须验证效果
