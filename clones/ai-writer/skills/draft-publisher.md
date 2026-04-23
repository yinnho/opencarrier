---
name: draft-publisher
when_to_use: 当文章排完版需要发布到草稿箱，或用户说"发布"、"发到草稿箱"、"推送到公众号"时
allowed_tools: ["mcp_wechat_oa_get_access_token", "mcp_wechat_oa_upload_media", "mcp_wechat_oa_create_draft", "mcp_wechat_oa_list_drafts", "mcp_wechat_oa_publish_draft"]
version: 1
usage_count: 0
---

# Draft Publisher

将排版好的文章发布到微信公众号草稿箱。

## Process

### 1. 获取凭证

从 knowledge/product-guide.md 中读取公众号的 `app_id` 和 `app_secret`。

如果没有配置，提示用户：
> 请提供公众号的 AppID 和 AppSecret，我会记录到产品指南中供后续使用。

### 2. 封面图处理（可选）

如果有封面图：
1. 将图片转为 base64
2. 调用 `mcp_wechat_oa_upload_media`（app_id, app_secret, media_type="image", filename, data_base64）
3. 获取返回的 `media_id` 作为 `thumb_media_id`

如果没有封面图，跳过此步骤（用户可在公众号后台手动添加）。

### 3. 创建草稿

调用 `mcp_wechat_oa_create_draft`：

必填参数：
- `app_id` — 公众号 AppID
- `app_secret` — 公众号 AppSecret
- `title` — 文章标题
- `content` — 排版后的 HTML

可选参数：
- `author` — 作者名
- `digest` — 摘要
- `thumb_media_id` — 封面图 media_id
- `content_source_url` — 原文链接

### 4. 确认结果

返回草稿创建结果，包括 `media_id`。

告知用户：
> 文章已保存到草稿箱。你可以在公众号后台预览和编辑。确认无误后说"发布"我会执行正式发布。

### 5. 正式发布（需用户确认）

**只有用户明确说"发布"时才执行此步骤。**

调用 `mcp_wechat_oa_publish_draft`（app_id, app_secret, media_id），提交发布。

## Important Principles

- **create_draft（创建草稿）可以自动执行**
- **publish_draft（正式发布）必须等用户确认**
- 每次调用 MCP 工具都要传入 app_id 和 app_secret（多租户设计）
- 发布前告知用户这是正式操作，发布后文章会对所有粉丝可见
- 如果 API 返回错误，翻译错误信息给用户，不要直接展示原始 JSON
