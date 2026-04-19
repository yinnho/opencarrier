---
name: skill-design
when_to_use: clone-trainer 或用户请求设计一个新的 skill
allowed_tools: ["agent_send"]
version: 1
usage_count: 0
---

# 技能设计

当收到 skill 设计请求时，根据需求生成完整的 skill markdown 文件。

## 输入

从请求中获取：
- **skill 名称**：英文短横线格式
- **用途描述**：解决什么问题
- **需要的工具**：执行时用到的内置工具列表
- **需要的 providers**：外部 API 依赖（可选，如 kling、openai）

## 输出格式

返回完整的 skill markdown，严格遵循以下格式：

```markdown
---
name: {skill-name}
when_to_use: {明确具体的触发条件，一句话描述什么时候该用这个 skill}
allowed_tools: [{工具列表}]
---

# {技能标题}

{技能 prompt 正文}

## 流程

### 1. {步骤1标题}
{具体操作步骤}

### 2. {步骤2标题}
{具体操作步骤}

## 重要原则
- {原则1}
- {原则2}
```

## 设计原则

1. **when_to_use 必须具体**：不能写"用户需要时使用"，要写明具体的触发场景
2. **allowed_tools 最小化**：只声明真正需要的工具，不要贪多
3. **providers 声明**：如果 skill 需要调用外部 API，在 `requirements` 部分添加：
   ```markdown
   ## Requirements
   - providers: ["{provider_name}"]
   ```
   这样运行时系统会自动注入对应的 API 密钥到环境变量
4. **步骤可操作**：每一步都要明确做什么、用什么工具、传什么参数
5. **边界清晰**：说明 skill 不做什么，避免过度执行

## 返回

直接返回完整的 markdown 内容，不要附加解释。clone-trainer 会将它写入目标分身的 skills/ 目录。
