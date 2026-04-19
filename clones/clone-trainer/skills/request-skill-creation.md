---
name: request-skill-creation
when_to_use: 其他分身通过 agent_send 请求创建新的 skill，或者用户要求为某个分身创建新 skill
allowed_tools: ["agent_send", "train_read", "train_write", "train_list"]
version: 1
usage_count: 0
---

# 技能创建请求处理

当收到其他分身或用户的 skill 创建请求时，按以下流程执行。

## 流程

### 1. 收集需求

从请求消息中提取：
- **请求方分身**：谁需要这个 skill（target 名称）
- **skill 名称**：英文短横线格式（如 web-search）
- **用途描述**：这个 skill 要解决什么问题
- **需要的工具**：skill 执行时需要哪些内置工具
- **需要的 providers**：skill 是否需要外部 API（如 kling、openai）

如果信息不完整，通过 `agent_send` 向请求方追问。

### 2. 委托 skill-creator 生成

使用 `agent_send` 向 `clone-creator` 发送消息：

```
请帮我设计一个 skill，要求如下：
- 名称：{skill_name}
- 用途：{description}
- 需要的工具：{tools}
- 需要的 providers：{providers}（可选）
```

等待 clone-creator 返回 skill 的完整 markdown 内容。

### 3. 安装到请求方

拿到 skill 内容后，使用 `train_write` 写入请求方的 workspace：

```
train_write({
  "target": "{请求方名称}",
  "path": "skills/{skill_name}.md",
  "content": "{skill 的完整 markdown 内容}"
})
```

### 4. 通知请求方

安装完成后，通过 `agent_send` 通知请求方：

```
skill "{skill_name}" 已安装到你的 skills/ 目录。
你下次执行任务时就能使用这个 skill 了。
```

## 重要原则

- 你只负责协调，实际的 skill 设计由 clone-creator 完成
- 写入目标一定是请求方的 workspace，不是你自己的
- 确认 skill 内容包含完整的 frontmatter（name, when_to_use, allowed_tools）
- 如果 skill 需要 providers，提醒请求方确保 brain.json 中有对应的 provider 配置
