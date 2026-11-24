use super::RpcError;
use serde_json::{json, Map, Value};

struct Tool {
    name: &'static str,
    description: &'static str,
    fields: &'static [(&'static str, &'static str, bool)],
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "register_agent",
        description: "使用用户指定的名称和团队注册当前 Antigravity.",
        fields: &[("name", "string", true), ("team", "string", true)],
    },
    Tool {
        name: "reconnect",
        description: "恢复当前 Antigravity 的 xats 身份.",
        fields: &[],
    },
    Tool {
        name: "get_inbox",
        description: "读取当前身份的新邮件.",
        fields: &[("limit", "integer", false)],
    },
    Tool {
        name: "send_message",
        description: "给 xats 队友发私信. 跨团队回信时, 从原邮件取 from_name 和 from_team 分别传入 to_agent_name 和 to_team. body 为正文.",
        fields: &[
            ("to_agent_name", "string", true),
            ("to_team", "string", false),
            ("subject", "string", false),
            ("body", "string", true),
            ("need_reply", "boolean", false),
            ("auto_poke", "boolean", false),
            ("await_ack_s", "integer", false),
        ],
    },
    Tool {
        name: "send_message_by_id",
        description: "给同团队的成员按 ID 回信. 使用邮件的 from_agent_id 作为 to_agent_id. 跨团队请使用 send_message 并指定 to_team.",
        fields: &[
            ("to_agent_id", "string", true),
            ("body", "string", true),
            ("subject", "string", false),
            ("need_reply", "boolean", false),
            ("auto_poke", "boolean", false),
            ("await_ack_s", "integer", false),
        ],
    },
    Tool {
        name: "list_agents",
        description: "列出本团队的成员.",
        fields: &[],
    },
    Tool {
        name: "broadcast",
        description: "发送团队广播.",
        fields: &[
            ("subject", "string", false),
            ("body", "string", true),
            ("auto_poke", "boolean", false),
        ],
    },
    Tool {
        name: "broadcast_to_role",
        description: "向本团队指定角色的所有成员发送广播.",
        fields: &[
            ("to_role", "string", true),
            ("subject", "string", false),
            ("body", "string", true),
            ("auto_poke", "boolean", false),
        ],
    },
    Tool {
        name: "get_delivery_status",
        description: "查询自己发出邮件的唤醒投递与已读回执状态.",
        fields: &[("message_id", "string", true)],
    },
    Tool {
        name: "cancel_unread_alert",
        description: "取消自己发出邮件的未读提醒.",
        fields: &[
            ("message_id", "string", true),
            ("same_recipient_pending", "boolean", false),
        ],
    },
];

pub(super) fn communication(name: &str) -> bool {
    matches!(
        name,
        "get_inbox"
            | "list_agents"
            | "send_message"
            | "send_message_by_id"
            | "broadcast"
            | "broadcast_to_role"
            | "get_delivery_status"
            | "cancel_unread_alert"
    )
}

pub(super) fn list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            let mut properties = Map::new();
            let mut required = Vec::new();
            for &(field, kind, mandatory) in tool.fields {
                properties.insert(field.into(), json!({"type":kind}));
                if mandatory {
                    required.push(field);
                }
            }
            json!({"name":tool.name, "description":tool.description, "inputSchema":{
            "type":"object", "properties":properties, "required":required,
            "additionalProperties":false}})
        })
        .collect()
}

fn arguments(params: &Value) -> Result<(&'static Tool, Map<String, Value>), RpcError> {
    let tool = TOOLS
        .iter()
        .find(|t| params["name"].as_str() == Some(t.name))
        .ok_or_else(|| RpcError::invalid("Unknown Antigravity tool"))?;
    let args = match params.get("arguments") {
        None => Map::new(),
        Some(Value::Object(obj)) => obj.clone(),
        _ => return Err(RpcError::invalid("Expected a tool arguments object")),
    };
    if args
        .keys()
        .any(|key| !tool.fields.iter().any(|f| f.0 == key))
    {
        let allowed = tool.fields.iter().map(|field| field.0).collect::<Vec<_>>();
        return Err(RpcError::invalid(format!(
            "Unsupported argument for {}. Allowed fields: [{}]",
            tool.name,
            allowed.join(", ")
        )));
    }
    for &(name, kind, required) in tool.fields {
        let valid = match args.get(name) {
            None => !required,
            Some(v) => match kind {
                "string" => v
                    .as_str()
                    .is_some_and(|s| name == "subject" || !s.trim().is_empty()),
                "integer" => v.as_i64().is_some_and(|n| {
                    if name == "await_ack_s" {
                        (0..=30).contains(&n)
                    } else {
                        n > 0
                    }
                }),
                "boolean" => v.is_boolean(),
                _ => false,
            },
        };
        if !valid {
            return Err(RpcError::invalid(format!(
                "Invalid or missing argument: {name}"
            )));
        }
    }
    Ok((tool, args))
}

pub(super) fn prepare_communication(params: &Value) -> Result<Value, RpcError> {
    if !communication(params["name"].as_str().unwrap_or_default()) {
        return Err(RpcError::invalid("Tool unavailable in named Antigravity"));
    }
    let (tool, args) = arguments(params)?;
    Ok(json!({"name":tool.name, "arguments":args}))
}

pub(super) fn prepare(params: &Value, key: &str) -> Result<Value, RpcError> {
    let (tool, mut args) = arguments(params)?;
    if matches!(tool.name, "register_agent" | "reconnect") {
        args.insert("identity_key".into(), json!(key));
        if tool.name == "register_agent" {
            args.insert("agent_type".into(), json!("custom"));
            args.insert("agent_type_name".into(), json!("Antigravity"));
        }
    }
    Ok(json!({"name":tool.name, "arguments":args}))
}
