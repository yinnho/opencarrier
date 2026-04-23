---
name: hot-topic-finder
when_to_use: 当用户说"找热点"、"有什么值得写的"、"今日AI动态"、"帮我选题"时
allowed_tools: ["web_search", "web_fetch"]
version: 1
usage_count: 0
---

# Hot Topic Finder

当需要发现 AI 领域值得写的话题时，执行以下流程。

## Process

### 1. 多源搜索

使用 web_search 同时搜索以下方向：

- **AI 产品发布**: 搜索 "AI release" "AI launch" "大模型发布"（中英文各搜一次）
- **AI 行业动态**: 搜索 "AI industry news" "AI 融资" "AI 政策"
- **技术突破**: 搜索 "AI research breakthrough" "LLM benchmark" "Agent framework"
- **OpenCarrier/Aginx 相关**: 搜索 "MCP protocol" "AI Agent OS" "agent framework"

### 2. 筛选评估

对每个候选话题，按以下标准评分：

| 维度 | 权重 |
|------|------|
| 时效性（24h 内） | 30% |
| 读者关注度 | 25% |
| 与我们产品的关联度 | 25% |
| 可写性（素材是否充分） | 20% |

### 3. 输出选题

输出 3-5 个推荐选题，每个包含：
- **标题**: 拟定文章标题
- **类型**: 产品文章 / 行业分析 / 热点评论
- **理由**: 一句话说明为什么值得写
- **角度**: 写作的切入点

等用户选择后再进入写作流程。

## Important Principles

- 搜索结果只作为参考，文章必须原创
- 优先选择与我们产品领域相关的选题
- 避免"冷饭热炒"——检查是否已是广泛传播的旧闻
