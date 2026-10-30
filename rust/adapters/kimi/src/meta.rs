//! AgentLens fork：按会话补读真实工作目录和标题。

use std::{collections::HashMap, fs, path::PathBuf};

use ccusage_adapter_common::session_meta::{SessionMeta, clip_title, project_root};
use ccusage_core::LoadedEntry;
use serde_json::Value;

use crate::paths::paths;

/// 键为 sessionId（会话目录名）。根目录的 `session_index.jsonl` 记录 `sessionDir` 和 `workDir`；
/// 会话目录下的 `state.json` 带 `cwd` 和 `title`，旧会话可能只有其中一部分。
pub fn session_meta(entries: &[LoadedEntry]) -> HashMap<String, SessionMeta> {
    let mut index: HashMap<String, (PathBuf, Option<String>)> = HashMap::new();
    for root in paths().unwrap_or_default() {
        let Ok(text) = fs::read_to_string(root.join("session_index.jsonl")) else {
            continue;
        };
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let field = |key| value.get(key).and_then(Value::as_str).map(str::to_string);
            if let (Some(id), Some(dir)) = (field("sessionId"), field("sessionDir")) {
                index.insert(id, (PathBuf::from(dir), field("workDir")));
            }
        }
    }
    let mut result = HashMap::new();
    for entry in entries {
        let session_id = &*entry.session_id;
        if result.contains_key(session_id) {
            continue;
        }
        let Some((dir, work_dir)) = index.get(session_id) else {
            continue;
        };
        let state = fs::read_to_string(dir.join("state.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        let state_field = |key| state.as_ref()?.get(key)?.as_str();
        let cwd = state_field("cwd").or(work_dir.as_deref());
        let meta = SessionMeta {
            project: cwd.map(project_root),
            // 新建会话在首条消息前的占位标题。
            title: state_field("title")
                .filter(|title| *title != "New Session")
                .and_then(clip_title),
            ..SessionMeta::default()
        };
        result.insert(session_id.to_string(), meta);
    }
    result
}
