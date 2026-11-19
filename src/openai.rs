//! 纯转换层：OpenAI 数据结构 <-> driver 需要的文本/JSON。
//! 这里没有任何 I/O 或共享状态，可以单独读写、单独测试。

use serde::Deserialize;
use serde_json::{json, Value};

// serde 默认会忽略未知字段，所以 temperature / max_tokens 等不用声明。
// #[serde(default)] = 请求里缺这个字段时用 Default::default()，避免反序列化失败。
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub model: Option<String>,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub stream: bool,
}

// content 在 OpenAI 协议里可能是字符串，也可能是 [{type:"text",text:"..."}] 数组，
// 所以先收成 Value，交给 content_text() 再归一化。
#[derive(Debug, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Option<Value>,
    // 工具结果消息会带这两个字段；网页端拿不到原生 id，只能拼进文本里告诉模型
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

// 模型 id 只是“选择器”，真正的模式由 driver 的 applyModel / 校准点击决定
pub fn mode_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.contains("reason") || m.contains("expert") || m.contains("r1") || m.contains("think") {
        "expert"
    } else {
        "quick"
    }
}

// 把两种 content 形态统一成纯文本；非文本 part（图片等）直接丢掉
fn content_text(content: &Option<Value>) -> String {
    match content {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| {
                let kind = p.get("type").and_then(|v| v.as_str());
                if kind == Some("text") {
                    p.get("text").and_then(|v| v.as_str()).map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
    }
}

// 只发最后一条消息：网页端页面自己保留历史，发全量历史容易超长导致发送失败
pub fn build_question(req: &ChatRequest) -> String {
    let last = match req.messages.last() {
        Some(m) => m,
        None => return "(no message)".to_string(),
    };
    let text = content_text(&last.content);
    match last.role.as_str() {
        "user" => format!("[用户]\n{text}"),
        "assistant" => format!("[助手]\n{text}"),
        // 工具结果务必带 id/name 并显式要求不要重复调用，否则网页模型会原地重放同一个工具
        "tool" => {
            let id = last.tool_call_id.as_deref().unwrap_or("-");
            let name = last.name.as_deref().unwrap_or("-");
            format!(
                "[工具结果 id={id} name={name}]\n工具 {name} 已经执行完成，不要再次调用同一个工具。请基于下面的结果继续下一步，或直接给出最终回答：\n{text}"
            )
        }
        "system" => format!("[系统指令]\n{text}"),
        other => format!("[{other}]\n{text}"),
    }
}

// 网页版没有原生 function calling，只能用提示词让它输出约定的 TYC 代码块（内容为 YAML），
// 再由 driver 侧的多层容错解析成结构化的 toolCalls。
pub fn build_tools_text(tools: &[Value]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let fence = "```";
    let mut lines = vec![
        "你可以调用工具来完成用户任务。工具会自动执行，不需要担心权限问题——直接调用即可。".to_string(),
        String::new(),
        "当你需要调用工具时，你的整个回复必须是一个 TYC 代码块（标签为 TYC，内容为 YAML）——前后不要有任何文字、解释或标点："
            .to_string(),
        String::new(),
        format!("{fence}TYC"),
        "name: 工具名".to_string(),
        "args:".to_string(),
        "  参数1: 值1".to_string(),
        fence.to_string(),
        String::new(),
        "关键规则：".to_string(),
        "- 调用工具时整个回复 ONLY 代码块（无散文）。任务完成后才用纯文本回答。".to_string(),
        "- 一次只调用一个工具。".to_string(),
        "- 代码块标签必须是 TYC，内容是合法 YAML。".to_string(),
        "- 必须包含 name 和 args 两个键。".to_string(),
        "- 字符串值如果包含冒号、井号或换行，请用双引号包裹。".to_string(),
        "- 收到“工具结果”后，说明该工具已经执行完成：绝不要重复调用同一个工具，必须基于结果继续或直接给出最终回答。".to_string(),
        String::new(),
        "可用工具：".to_string(),
    ];
    for tool in tools {
        let f = tool.get("function").unwrap_or(tool);
        let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let desc = f.get("description").and_then(|v| v.as_str()).unwrap_or("");
        lines.push(format!("- {name}: {desc}"));
        if let Some(params) = f.get("parameters") {
            lines.push(format!(
                "  参数 schema (JSON): {}",
                serde_json::to_string(params).unwrap_or_default()
            ));
        }
    }
    lines.join("\n")
}


/// 从模型回复中提取 ```TYC 代码块并解析其中的 YAML
pub fn parse_tool_call_yaml(text: &str) -> Option<Value> {
    // 1. 找 ```TYC ... ``` 块（标签大小写不敏感）
    let body = extract_tyc_block(text)?;

    // 2. 解析 YAML -> serde_json::Value
    //    serde_yaml 的 Value 和 serde_json::Value 不通用，需要中转
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(body).ok()?;
    let json_val: Value = serde_json::to_value(yaml_val).ok()?;

    // 3. 校验必须有 name / args
    if json_val.get("name").is_none() || json_val.get("args").is_none() {
        return None;
    }
    Some(json_val)
}

// 扫过所有围栏代码块，返回第一个标签为 TYC 的块内容（已 trim）。
// 不直接 find("```TYC")：模型可能在 ``` 和标签之间带空格，或标签大小写不一致。
fn extract_tyc_block(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(pos) = text[from..].find("```") {
        let fence_start = from + pos;
        let after = &text[fence_start + 3..];
        let line_end = after.find('\n')?;
        let tag = after[..line_end].trim();
        let body_start = fence_start + 3 + line_end + 1;
        if tag.eq_ignore_ascii_case("TYC") {
            let rest = &text[body_start..];
            let end = rest.find("```")?;
            return Some(rest[..end].trim());
        }
        from = fence_start + 3;
    }
    None
}
// 按字符切而不是按字节切，否则多字节 UTF-8 会被切开变成乱码
pub fn split_chars(text: &str, size: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(size.max(1))
        .map(|c| c.iter().collect::<String>())
        .collect()
}

// driver 的 `models` RPC 不可用时的兜底列表（按站点）。
pub fn fallback_models(site: &str, created: u64) -> Vec<Value> {
    let list: &[(&str, &str)] = match site {
        "qwen" => &[
            ("qwen3.7-plus", "Qwen 3.7 Plus"),
            ("qwen3.8-max", "Qwen 3.8 Max"),
        ],
        "doubao" => &[
            ("doubao-pro", "快速"),
            ("doubao-think", "深度思考"),
            ("doubao-expert", "专家"),
        ],
        "gpt" => &[
            ("auto", "Auto"),
            ("gpt-5-5", "GPT-5.5"),
            ("gpt-5-4-thinking", "GPT-5.4 Thinking"),
            ("gpt-5-3-mini", "GPT-5.3 Mini"),
            ("gpt-5-mini", "GPT-5 mini"),
        ],
        _ => &[
            ("deepseek-chat", "DeepSeek 快速"),
            ("deepseek-reasoner", "DeepSeek 专家"),
        ],
    };
    list.iter()
        .map(|(id, name)| {
            json!({
                "id": id,
                "object": "model",
                "created": created,
                "owned_by": format!("{site}-web"),
                "name": name
            })
        })
        .collect()
}
