//! Hardcoded tool schemas for all MCP tools.
//!
//! Source: wecom-cli skills/ directory definitions.

use serde_json::Value;

/// A tool specification: name, category, description, parameter schema.
pub struct ToolSpec {
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

/// All MCP tools organized by category.
pub fn all_tools() -> Vec<ToolSpec> {
    let mut tools = Vec::new();
    tools.extend(contact_tools());
    tools.extend(doc_tools());
    tools.extend(msg_tools());
    tools.extend(todo_tools());
    tools.extend(meeting_tools());
    tools.extend(schedule_tools());
    tools
}

// ---------------------------------------------------------------------------
// Contact (1 tool)
// ---------------------------------------------------------------------------

fn contact_tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "get_userlist",
        category: "contact",
        description: "获取当前用户可见范围内的通讯录成员列表，返回 userid、姓名和别名",
        schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
    }]
}

// ---------------------------------------------------------------------------
// Doc (18 tools)
// ---------------------------------------------------------------------------

fn doc_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_doc_content",
            category: "doc",
            description: "获取文档内容（支持普通文档和智能文档），异步接口需轮询 task_id",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "url": { "type": "string", "description": "文档URL（与docid二选一）" },
                    "type": { "type": "integer", "description": "内容格式：2=Markdown（默认）", "default": 2 },
                    "task_id": { "type": "string", "description": "异步轮询的任务ID" }
                }
            }),
        },
        ToolSpec {
            name: "create_doc",
            category: "doc",
            description: "创建企业微信文档或智能表格",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "doc_type": { "type": "integer", "description": "文档类型：3=文档, 10=智能表格" },
                    "doc_name": { "type": "string", "description": "文档名称" }
                },
                "required": ["doc_type", "doc_name"]
            }),
        },
        ToolSpec {
            name: "edit_doc_content",
            category: "doc",
            description: "编辑文档内容（覆盖写入，content_type=1 为 Markdown 格式）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "url": { "type": "string", "description": "文档URL（与docid二选一）" },
                    "content": { "type": "string", "description": "文档内容（Markdown格式）" },
                    "content_type": { "type": "integer", "description": "内容格式：1=Markdown", "default": 1 }
                },
                "required": ["content"]
            }),
        },
        ToolSpec {
            name: "smartpage_export_task",
            category: "doc",
            description: "导出智能文档（创建异步导出任务）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "url": { "type": "string", "description": "文档URL（与docid二选一）" },
                    "content_type": { "type": "integer", "description": "导出格式：1=Markdown", "default": 1 }
                }
            }),
        },
        ToolSpec {
            name: "smartpage_get_export_result",
            category: "doc",
            description: "获取智能文档导出结果（轮询异步任务）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "导出任务ID" }
                },
                "required": ["task_id"]
            }),
        },
        // Smart Sheet tools
        ToolSpec {
            name: "smartsheet_get_sheet",
            category: "doc",
            description: "获取智能表格的所有子表列表",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" }
                },
                "required": ["docid"]
            }),
        },
        ToolSpec {
            name: "smartsheet_add_sheet",
            category: "doc",
            description: "在智能表格中添加子表",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "properties": {
                        "type": "object",
                        "properties": { "title": { "type": "string", "description": "子表标题" } },
                        "required": ["title"]
                    }
                },
                "required": ["docid", "properties"]
            }),
        },
        ToolSpec {
            name: "smartsheet_update_sheet",
            category: "doc",
            description: "更新智能表格子表属性（重命名）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "properties": {
                        "type": "object",
                        "properties": {
                            "sheet_id": { "type": "string", "description": "子表ID" },
                            "title": { "type": "string", "description": "新标题" }
                        },
                        "required": ["sheet_id", "title"]
                    }
                },
                "required": ["docid", "properties"]
            }),
        },
        ToolSpec {
            name: "smartsheet_delete_sheet",
            category: "doc",
            description: "删除智能表格子表（不可恢复）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" }
                },
                "required": ["docid", "sheet_id"]
            }),
        },
        ToolSpec {
            name: "smartsheet_get_fields",
            category: "doc",
            description: "获取智能表格子表的所有字段定义",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" }
                },
                "required": ["docid", "sheet_id"]
            }),
        },
        ToolSpec {
            name: "smartsheet_add_fields",
            category: "doc",
            description: "在智能表格子表中添加字段",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "fields": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "field_title": { "type": "string", "description": "字段标题" },
                                "field_type": { "type": "integer", "description": "字段类型" }
                            },
                            "required": ["field_title", "field_type"]
                        },
                        "description": "字段列表（最多150个）"
                    }
                },
                "required": ["docid", "sheet_id", "fields"]
            }),
        },
        ToolSpec {
            name: "smartsheet_update_fields",
            category: "doc",
            description: "更新智能表格字段（仅支持重命名）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "fields": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "field_id": { "type": "string", "description": "字段ID" },
                                "field_title": { "type": "string", "description": "新字段标题" },
                                "field_type": { "type": "integer", "description": "字段类型" }
                            },
                            "required": ["field_id", "field_title"]
                        }
                    }
                },
                "required": ["docid", "sheet_id", "fields"]
            }),
        },
        ToolSpec {
            name: "smartsheet_delete_fields",
            category: "doc",
            description: "删除智能表格字段（不可恢复）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "field_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要删除的字段ID列表"
                    }
                },
                "required": ["docid", "sheet_id", "field_ids"]
            }),
        },
        ToolSpec {
            name: "smartsheet_get_records",
            category: "doc",
            description: "获取智能表格子表的所有记录",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "url": { "type": "string", "description": "文档URL（与docid二选一）" },
                    "sheet_id": { "type": "string", "description": "子表ID" }
                },
                "required": ["sheet_id"]
            }),
        },
        ToolSpec {
            name: "smartsheet_add_records",
            category: "doc",
            description: "向智能表格子表添加记录（最多500行）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "records": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "values": {
                                    "type": "object",
                                    "description": "字段名到值的映射"
                                }
                            },
                            "required": ["values"]
                        },
                        "description": "记录列表（最多500条）"
                    }
                },
                "required": ["docid", "sheet_id", "records"]
            }),
        },
        ToolSpec {
            name: "smartsheet_update_records",
            category: "doc",
            description: "更新智能表格记录（最多500行）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "key_type": { "type": "string", "description": "匹配方式：字段标题或字段ID" },
                    "records": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "record_id": { "type": "string", "description": "记录ID" },
                                "values": { "type": "object", "description": "要更新的字段值" }
                            },
                            "required": ["record_id", "values"]
                        }
                    }
                },
                "required": ["docid", "sheet_id", "records"]
            }),
        },
        ToolSpec {
            name: "smartsheet_delete_records",
            category: "doc",
            description: "删除智能表格记录（不可恢复，最多500条）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "docid": { "type": "string", "description": "文档ID" },
                    "sheet_id": { "type": "string", "description": "子表ID" },
                    "record_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要删除的记录ID列表（最多500条）"
                    }
                },
                "required": ["docid", "sheet_id", "record_ids"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Msg (3 tools — excluding get_msg_media)
// ---------------------------------------------------------------------------

fn msg_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_msg_chat_list",
            category: "msg",
            description: "获取会话列表，支持按时间范围分页查询",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "begin_time": { "type": "string", "description": "开始时间 YYYY-MM-DD" },
                    "end_time": { "type": "string", "description": "结束时间 YYYY-MM-DD" },
                    "cursor": { "type": "string", "description": "分页游标" }
                },
                "required": ["begin_time", "end_time"]
            }),
        },
        ToolSpec {
            name: "get_message",
            category: "msg",
            description: "拉取指定会话的消息记录（支持文本/图片/文件/语音/视频，7天回溯）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "chat_type": { "type": "integer", "description": "会话类型：1=单聊, 2=群聊" },
                    "chatid": { "type": "string", "description": "会话ID" },
                    "begin_time": { "type": "string", "description": "开始时间 YYYY-MM-DD HH:mm:ss" },
                    "end_time": { "type": "string", "description": "结束时间 YYYY-MM-DD HH:mm:ss" },
                    "cursor": { "type": "string", "description": "分页游标" }
                },
                "required": ["chat_type", "chatid", "begin_time", "end_time"]
            }),
        },
        ToolSpec {
            name: "send_message",
            category: "msg",
            description: "发送文本消息（单聊或群聊，最多2048字节）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "chat_type": { "type": "integer", "description": "会话类型：1=单聊, 2=群聊" },
                    "chatid": { "type": "string", "description": "会话ID" },
                    "msgtype": { "type": "string", "description": "消息类型，目前仅支持 text", "default": "text" },
                    "text": {
                        "type": "object",
                        "properties": { "content": { "type": "string", "description": "消息内容（最多2048字节）" } },
                        "required": ["content"]
                    }
                },
                "required": ["chat_type", "chatid", "text"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Todo (6 tools)
// ---------------------------------------------------------------------------

fn todo_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_todo_list",
            category: "todo",
            description: "查询待办列表，支持按创建时间和提醒时间筛选",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "create_begin_time": { "type": "string", "description": "创建开始时间" },
                    "create_end_time": { "type": "string", "description": "创建结束时间" },
                    "remind_begin_time": { "type": "string", "description": "提醒开始时间" },
                    "remind_end_time": { "type": "string", "description": "提醒结束时间" },
                    "limit": { "type": "integer", "description": "返回数量上限（最多20）", "default": 20 },
                    "cursor": { "type": "string", "description": "分页游标" }
                }
            }),
        },
        ToolSpec {
            name: "get_todo_detail",
            category: "todo",
            description: "获取待办事项详情（支持批量，最多20个）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "todo_id_list": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "待办ID列表（最多20个）"
                    }
                },
                "required": ["todo_id_list"]
            }),
        },
        ToolSpec {
            name: "create_todo",
            category: "todo",
            description: "创建待办事项",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "待办内容" },
                    "follower_list": {
                        "type": "object",
                        "properties": {
                            "followers": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "follower_id": { "type": "string", "description": "跟进人userid" },
                                        "follower_status": { "type": "integer", "description": "状态：0=拒绝, 1=接受, 2=完成" }
                                    }
                                }
                            }
                        }
                    },
                    "remind_time": { "type": "string", "description": "提醒时间" }
                },
                "required": ["content"]
            }),
        },
        ToolSpec {
            name: "update_todo",
            category: "todo",
            description: "更新待办事项",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string", "description": "待办ID" },
                    "content": { "type": "string", "description": "新内容" },
                    "follower_list": { "type": "object", "description": "跟进人列表（全量替换）" },
                    "todo_status": { "type": "integer", "description": "状态：0=完成, 1=进行中, 2=删除" },
                    "remind_time": { "type": "string", "description": "提醒时间" }
                },
                "required": ["todo_id"]
            }),
        },
        ToolSpec {
            name: "delete_todo",
            category: "todo",
            description: "删除待办事项（不可恢复）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string", "description": "待办ID" }
                },
                "required": ["todo_id"]
            }),
        },
        ToolSpec {
            name: "change_todo_user_status",
            category: "todo",
            description: "变更用户待办处理状态",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string", "description": "待办ID" },
                    "user_status": { "type": "integer", "description": "状态：0=拒绝, 1=接受, 2=完成" }
                },
                "required": ["todo_id", "user_status"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Meeting (5 tools)
// ---------------------------------------------------------------------------

fn meeting_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "create_meeting",
            category: "meeting",
            description: "创建预约会议",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "会议标题" },
                    "meeting_start_datetime": { "type": "string", "description": "开始时间 YYYY-MM-DD HH:mm" },
                    "meeting_duration": { "type": "integer", "description": "会议时长（秒）" },
                    "description": { "type": "string", "description": "会议描述" },
                    "location": { "type": "string", "description": "会议地点" },
                    "invitees": {
                        "type": "object",
                        "properties": {
                            "userid": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "受邀人userid列表"
                            }
                        }
                    },
                    "settings": {
                        "type": "object",
                        "description": "会议设置（密码、等候室、入会静音等）"
                    }
                },
                "required": ["title", "meeting_start_datetime", "meeting_duration"]
            }),
        },
        ToolSpec {
            name: "list_user_meetings",
            category: "meeting",
            description: "查询用户的会议列表（前后30天）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "begin_datetime": { "type": "string", "description": "开始时间" },
                    "end_datetime": { "type": "string", "description": "结束时间" },
                    "cursor": { "type": "string", "description": "分页游标" },
                    "limit": { "type": "integer", "description": "返回数量（最多100）" }
                },
                "required": ["begin_datetime", "end_datetime"]
            }),
        },
        ToolSpec {
            name: "get_meeting_info",
            category: "meeting",
            description: "获取会议详情",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "meetingid": { "type": "string", "description": "会议ID" },
                    "meeting_code": { "type": "string", "description": "会议码" },
                    "sub_meetingid": { "type": "string", "description": "子会议ID" }
                },
                "required": ["meetingid"]
            }),
        },
        ToolSpec {
            name: "cancel_meeting",
            category: "meeting",
            description: "取消会议",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "meetingid": { "type": "string", "description": "会议ID" }
                },
                "required": ["meetingid"]
            }),
        },
        ToolSpec {
            name: "set_invite_meeting_members",
            category: "meeting",
            description: "更新会议受邀成员（全量替换）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "meetingid": { "type": "string", "description": "会议ID" },
                    "invitees": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "userid": { "type": "string" } },
                            "required": ["userid"]
                        },
                        "description": "受邀人列表（全量替换）"
                    }
                },
                "required": ["meetingid", "invitees"]
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Schedule (8 tools)
// ---------------------------------------------------------------------------

fn schedule_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "get_schedule_list_by_range",
            category: "schedule",
            description: "查询日程ID列表（前后30天）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "start_time": { "type": "string", "description": "开始时间（Unix时间戳或YYYY-MM-DD）" },
                    "end_time": { "type": "string", "description": "结束时间（Unix时间戳或YYYY-MM-DD）" }
                },
                "required": ["start_time", "end_time"]
            }),
        },
        ToolSpec {
            name: "get_schedule_detail",
            category: "schedule",
            description: "获取日程详情（支持批量，1-50个）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule_id_list": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "日程ID列表（1-50个）"
                    }
                },
                "required": ["schedule_id_list"]
            }),
        },
        ToolSpec {
            name: "create_schedule",
            category: "schedule",
            description: "创建日程",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule": {
                        "type": "object",
                        "properties": {
                            "start_time": { "type": "integer", "description": "开始时间（Unix时间戳）" },
                            "end_time": { "type": "integer", "description": "结束时间（Unix时间戳）" },
                            "summary": { "type": "string", "description": "日程标题" },
                            "description": { "type": "string", "description": "日程描述" },
                            "location": { "type": "string", "description": "地点" },
                            "is_whole_day": { "type": "integer", "description": "是否全天：0=否, 1=是" },
                            "attendees": {
                                "type": "array",
                                "items": { "type": "object", "properties": { "userid": { "type": "string" } } }
                            },
                            "reminders": {
                                "type": "object",
                                "properties": {
                                    "is_remind": { "type": "integer", "description": "是否提醒" },
                                    "remind_before_event_secs": { "type": "integer", "description": "提前多少秒提醒" },
                                    "timezone": { "type": "integer", "description": "时区（默认8=中国）" }
                                }
                            }
                        },
                        "required": ["start_time", "end_time", "summary"]
                    }
                },
                "required": ["schedule"]
            }),
        },
        ToolSpec {
            name: "update_schedule",
            category: "schedule",
            description: "修改日程（仅传需要修改的字段）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule": {
                        "type": "object",
                        "properties": {
                            "schedule_id": { "type": "string", "description": "日程ID" },
                            "start_time": { "type": "integer", "description": "开始时间" },
                            "end_time": { "type": "integer", "description": "结束时间" },
                            "summary": { "type": "string", "description": "标题" },
                            "description": { "type": "string", "description": "描述" },
                            "location": { "type": "string", "description": "地点" }
                        },
                        "required": ["schedule_id"]
                    }
                },
                "required": ["schedule"]
            }),
        },
        ToolSpec {
            name: "cancel_schedule",
            category: "schedule",
            description: "取消日程",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule_id": { "type": "string", "description": "日程ID" }
                },
                "required": ["schedule_id"]
            }),
        },
        ToolSpec {
            name: "add_schedule_attendees",
            category: "schedule",
            description: "添加日程参与人",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule_id": { "type": "string", "description": "日程ID" },
                    "attendees": {
                        "type": "array",
                        "items": { "type": "object", "properties": { "userid": { "type": "string" } } },
                        "description": "参与人列表"
                    }
                },
                "required": ["schedule_id", "attendees"]
            }),
        },
        ToolSpec {
            name: "del_schedule_attendees",
            category: "schedule",
            description: "移除日程参与人",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule_id": { "type": "string", "description": "日程ID" },
                    "attendees": {
                        "type": "array",
                        "items": { "type": "object", "properties": { "userid": { "type": "string" } } },
                        "description": "要移除的参与人列表"
                    }
                },
                "required": ["schedule_id", "attendees"]
            }),
        },
        ToolSpec {
            name: "check_availability",
            category: "schedule",
            description: "查询用户闲忙状态（最多10人）",
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "check_user_list": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "用户userid列表（1-10人）"
                    },
                    "start_time": { "type": "integer", "description": "开始时间（Unix时间戳）" },
                    "end_time": { "type": "integer", "description": "结束时间（Unix时间戳）" }
                },
                "required": ["check_user_list", "start_time", "end_time"]
            }),
        },
    ]
}
