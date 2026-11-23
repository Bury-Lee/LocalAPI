//! 会话历史：按调用 ID 保存 OpenAI messages，支持落盘与导出为 Markdown。
//!
//! 索引是 `BTreeMap<u64, Session>`，所以导出时能按 ID 稳定排序；
//! 消息保留原始 JSON 形态，导出时再统一渲染成文本。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 默认历史目录，每个会话保存为 `<id>.json`。
pub const DefaultHistoryPath: &str = "history";

/// 一次会话：api 调用 ID + 该次调用的消息列表。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// api 的调用 ID，同时是 `<id>.json` 的文件名
    #[serde(default)]
    pub id: u64,

    /// 创建时间（Unix 秒）
    #[serde(default)]
    pub created_at: u64,
    /// 使用的模型，空串表示未记录
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    /// 原始 OpenAI messages（role/content/tool_call_id/name 等）
    #[serde(default)]
    pub messages: Vec<Value>,
}

impl Session {
    pub fn new(id: u64) -> Self {
        Session {
            id,
            created_at: now_secs(),
            model: String::new(),
            messages: Vec::new(),
        }
    }

    /// 追加一条消息。
    pub fn push(&mut self, message: Value) {
        self.messages.push(message);
    }

    /// 导出为 Markdown 对话记录。
    pub fn ToMarkdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "# 会话 {}", self.id);
        let _ = writeln!(out);
        if self.created_at > 0 {
            let time = chrono::DateTime::from_timestamp(self.created_at as i64, 0)
                .map(|dt| {
                    dt.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_else(|| self.created_at.to_string());
            let _ = writeln!(out, "- 创建时间: {time}");
        }
        if !self.model.is_empty() {
            let _ = writeln!(out, "- 模型: {}", self.model);
        }
        if !self.model.is_empty() || self.created_at > 0 {
            let _ = writeln!(out);
        }

        for message in &self.messages {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let _ = writeln!(out, "## {}", role_label(role));
            if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                let _ = writeln!(out, "> tool_call_id: {id}");
            }
            if let Some(name) = message.get("name").and_then(Value::as_str) {
                let _ = writeln!(out, "> name: {name}");
            }
            let text = render_content(message.get("content"));
            if !text.is_empty() {
                let _ = writeln!(out, "{text}");
            }
            let _ = writeln!(out);
        }
        out.trim_end().to_string()
    }
}

/// 历史索引：内存里是 `BTreeMap`，`path` 指向落盘目录。
#[derive(Debug)]
pub struct History {
    path: PathBuf,
    map: BTreeMap<u64, Session>,
}

impl Default for History {
    fn default() -> Self {
        History::WithPath(DefaultHistoryPath)
    }
}

impl History {
    /// 指定落盘目录，初始为空；需要旧数据再调用 [`History::Load`]。
    pub fn WithPath(path: impl Into<PathBuf>) -> Self {
        History {
            path: path.into(),
            map: BTreeMap::new(),
        }
    }

    pub fn Path(&self) -> &Path {
        &self.path
    }

    /// 加载目录下所有 `<id>.json`；目录不存在视为空历史。
    /// 非 `<id>.json` 的文件会被忽略。
    pub fn Load(&mut self) -> Result<(), String> {
        self.map.clear();
        if !self.path.is_dir() {
            return Ok(());
        }
        let entries = fs::read_dir(&self.path)
            .map_err(|e| format!("读取历史目录失败 {}: {e}", self.path.display()))?;
        for entry in entries {
            let file = entry
                .map_err(|e| format!("遍历历史目录失败 {}: {e}", self.path.display()))?
                .path();
            if file.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = file
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
            else {
                continue;
            };
            let text = fs::read_to_string(&file)
                .map_err(|e| format!("读取会话文件失败 {}: {e}", file.display()))?;
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            if text.trim().is_empty() {
                continue;
            }
            let mut session: Session = serde_json::from_str(text)
                .map_err(|e| format!("解析会话文件失败 {}: {e}", file.display()))?;
            // 文件名才是映射表的 key，避免文件内容里的 id 写错导致索引不一致
            session.id = id;
            self.map.insert(id, session);
        }
        Ok(())
    }

    /// 新建会话并加入映射表；ID 已存在时报错。
    pub fn NewSession(&mut self, id: u64) -> Result<(), String> {
        if self.map.contains_key(&id) {
            return Err(format!("会话 {id} 已存在"));
        }
        self.map.insert(id, Session::new(id));
        Ok(())
    }

    /// 用一组消息覆盖某个会话；ID 不存在时自动新建，便于直接记录。
    pub fn Import(&mut self, id: u64, messages: Vec<Value>) -> Result<(), String> {
        match self.map.get_mut(&id) {
            Some(session) => session.messages = messages,
            None => {
                let mut session = Session::new(id);
                session.messages = messages;
                self.map.insert(id, session);
            }
        }
        Ok(())
    }

    /// 只有会话不存在时才导入，返回是否新建；用于“请求先落盘”且不能覆盖完整记录的场景。
    pub fn ImportIfAbsent(&mut self, id: u64, messages: Vec<Value>) -> Result<(), String> {
        if self.map.contains_key(&id) {
            return Ok(());
        }
        self.Import(id, messages)
    }

    /// 向指定会话追加一条消息；ID 不存在时自动新建。
    pub fn Append(&mut self, id: u64, message: Value) -> Result<(), String> {
        self.map
            .entry(id)
            .or_insert_with(|| Session::new(id))
            .push(message);
        Ok(())
    }

    pub fn Get(&self, id: u64) -> Option<&Session> {
        self.map.get(&id)
    }

    /// 记录会话使用的模型；会话不存在时忽略。
    pub fn SetModel(&mut self, id: u64, model: &str) {
        if let Some(session) = self.map.get_mut(&id) {
            session.model = model.to_string();
        }
    }

    pub fn Sessions(&self) -> &BTreeMap<u64, Session> {
        &self.map
    }

    /// 保存当前 History：每个会话写成一个 `<id>.json`。
    pub fn Save(&self) -> Result<(), String> {
        fs::create_dir_all(&self.path)
            .map_err(|e| format!("创建目录 {} 失败: {e}", self.path.display()))?;
        for session in self.map.values() {
            self.writeSession(session)?;
        }
        Ok(())
    }

    /// 只保存一个会话（用于追加后立即落盘）。
    pub fn SaveSession(&self, id: u64) -> Result<(), String> {
        let session = self
            .map
            .get(&id)
            .ok_or_else(|| format!("会话 {id} 不存在"))?;
        fs::create_dir_all(&self.path)
            .map_err(|e| format!("创建目录 {} 失败: {e}", self.path.display()))?;
        self.writeSession(session)
    }

    /// 先写临时文件再替换，避免中途失败留下半截文件。
    fn writeSession(&self, session: &Session) -> Result<(), String> {
        let file = self.path.join(format!("{}.json", session.id));
        let text = serde_json::to_string_pretty(session)
            .map_err(|e| format!("序列化会话 {} 失败: {e}", session.id))?;
        let tmp = file.with_extension("json.tmp");
        fs::write(&tmp, format!("{text}\n"))
            .map_err(|e| format!("写入临时文件 {} 失败: {e}", tmp.display()))?;
        if let Err(first) = fs::rename(&tmp, &file) {
            // 个别平台不允许 rename 覆盖已存在文件：删掉旧文件再试一次
            let _ = fs::remove_file(&file);
            fs::rename(&tmp, &file)
                .map_err(|e| format!("保存会话文件 {} 失败: {e}（{first}）", file.display()))?;
        }
        Ok(())
    }

    /// 导出一个历史记录为 Markdown,ID 不存在时将返回 `None`。
    pub fn OutportMarkdown(&self, id: u64) -> Option<String> {
        self.map.get(&id).map(Session::ToMarkdown)
    }
    /// 导出一个历史记录为 Json,ID 不存在时将返回 `None`。
    pub fn OutportJson(&self, id: u64) -> Option<String> {
        self.map
            .get(&id)
            .and_then(|session| serde_json::to_string_pretty(session).ok())
    }

    /// 导出全部历史（按 ID 升序），用分隔线拼接。
    pub fn OutPortAll(&self) -> String {
        self.map
            .values()
            .map(Session::ToMarkdown)
            .collect::<Vec<_>>()
            .join("\n\n---\n\n")
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn role_label(role: &str) -> String {
    match role {
        "user" => "用户".to_string(),
        "assistant" => "助手".to_string(),
        "system" => "系统".to_string(),
        "tool" => "工具".to_string(),
        other => other.to_string(),
    }
}

/// 把 OpenAI 的 content（字符串 / 多段数组 / null）统一成纯文本。
fn render_content(content: Option<&Value>) -> String {
    match content {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut texts = Vec::new();
            for part in parts {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            texts.push(text.to_string());
                        }
                    }
                    Some("image_url") | Some("image") => texts.push("[图片]".to_string()),
                    _ => {}
                }
            }
            texts.join("\n")
        }
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path(tag: &str) -> PathBuf {
        let unique = format!(
            "localapi-history-{tag}-{}-{}",
            std::process::id(),
            now_secs()
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn save_load_roundtrip() {
        let path = temp_path("roundtrip");
        let mut history = History::WithPath(&path);
        history.NewSession(1).unwrap();
        history
            .Import(
                1,
                vec![
                    json!({"role": "user", "content": "你好"}),
                    json!({"role": "assistant", "content": [{"type": "text", "text": "你好呀"}]}),
                ],
            )
            .unwrap();
        history
            .Import(2, vec![json!({"role": "user", "content": "第二个"})])
            .unwrap();
        history.Save().unwrap();

        assert!(path.join("1.json").is_file());
        assert!(path.join("2.json").is_file());

        let mut loaded = History::WithPath(&path);
        loaded.Load().unwrap();
        assert_eq!(loaded.Sessions().len(), 2);
        let session = loaded.Get(1).expect("session should exist");
        assert_eq!(session.messages.len(), 2);
        assert_eq!(loaded.OutportMarkdown(1).unwrap(), session.ToMarkdown());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn missing_file_is_empty_history() {
        let mut history = History::WithPath(temp_path("missing"));
        history.Load().unwrap();
        assert!(history.Sessions().is_empty());
    }

    #[test]
    fn load_ignores_non_id_files() {
        let path = temp_path("ignore");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("notes.json"), "{}").unwrap();
        fs::write(path.join("3.json.tmp"), "{}").unwrap();
        fs::write(
            path.join("3.json"),
            r#"{"id":3,"messages":[{"role":"user","content":"hi"}]}"#,
        )
        .unwrap();

        let mut history = History::WithPath(&path);
        history.Load().unwrap();
        assert_eq!(history.Sessions().len(), 1);
        assert!(history.Get(3).is_some());

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn save_session_writes_single_file() {
        let path = temp_path("single");
        let mut history = History::WithPath(&path);
        history
            .Import(9, vec![json!({"role": "user", "content": "only"})])
            .unwrap();
        history.SaveSession(9).unwrap();

        assert!(path.join("9.json").is_file());
        assert!(!path.join("10.json").exists());
        assert!(history.SaveSession(10).is_err());

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn append_creates_session_and_exports() {
        let mut history = History::WithPath(DefaultHistoryPath);
        history
            .Append(7, json!({"role": "user", "content": "记录我"}))
            .unwrap();
        let markdown = history.OutportMarkdown(7).unwrap();
        assert!(markdown.contains("用户"));
        assert!(markdown.contains("记录我"));
        assert!(history.OutportMarkdown(999).is_none());
    }

    #[test]
    fn new_session_rejects_duplicate() {
        let mut history = History::WithPath(DefaultHistoryPath);
        history.NewSession(2).unwrap();
        assert!(history.NewSession(2).is_err());
    }
}
