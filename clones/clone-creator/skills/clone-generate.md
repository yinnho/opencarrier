---
name: clone-generate
when_to_use: 用户要求创建一个新的分身，或描述了一个需要分身来完成的需求
allowed_tools: ["file_write", "file_read", "file_list", "shell_exec", "web_fetch"]
version: 1
usage_count: 0
---

# 分身生成技能

当用户表达了创建分身的意图时，执行以下流程：

## 流程

### 1. 需求收集

通过对话了解以下信息（不必一次问完，可以分步）：

- **分身名称**：英文短横线格式（如 customer-support）
- **用途描述**：一句话说清楚这个分身做什么
- **目标场景**：在什么场景下使用
- **人格特征**：什么性格、什么沟通风格
- **知识领域**：需要了解什么领域的知识
- **技能列表**：需要哪些能力（每个技能有触发条件）

### 2. 文件生成

信息收集完毕后，按以下结构生成文件：

```
/tmp/<clone-name>/
├── template.json
├── profile.md
├── SOUL.md
├── system_prompt.md
├── MEMORY.md
├── knowledge/
│   └── *.md
└── skills/
    └── *.md
```

使用 file_write 工具写入每个文件。

### 3. 打包

```bash
cd /tmp && tar czf <clone-name>.agx -C <clone-name> .
```

### 4. 安装

将 .agx 文件 base64 编码后通过 API 安装：

```bash
BASE64_DATA=$(base64 -i /tmp/<clone-name>.agx)
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64_DATA\"}"
```

或者如果需要指定用户：
```bash
curl -s -X POST http://localhost:4200/api/clones/install \
  -H "Content-Type: application/json" \
  -d "{\"data\": \"$BASE64_DATA\", \"user_id\": \"<user_id>\"}"
```

### 5. 确认

安装成功后告诉用户：
- 分身名称和 ID
- 可以通过 `/api/clones` 查看已安装分身
- 可以通过 `/api/clones/<name>/start` 启动分身

## 生成规则

- template.json 的 version 固定为 "1"
- profile.md 必须有 YAML frontmatter
- SOUL.md 用自然语言描述人格
- system_prompt.md 是最关键的文件，要详细且可操作
- 技能的 when_to_use 必须明确具体
- 知识文件按主题拆分，每个 1000-3000 字为宜
