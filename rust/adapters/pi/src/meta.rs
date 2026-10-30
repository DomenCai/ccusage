//! AgentLens fork：按会话补读真实工作目录、首条用户消息和每个模型走的 provider。

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

use ccusage_adapter_common::{
    collect_files_with_extension,
    session_meta::{SessionMeta, project_root, user_text},
};
use ccusage_core::LoadedEntry;
use serde_json::Value;

/// 键为 sessionId。`roots` 是该 store 的会话根目录（默认 store 用 `default_paths`）。
pub fn session_meta(roots: &[PathBuf], entries: &[LoadedEntry]) -> HashMap<String, SessionMeta> {
    let wanted: HashSet<&str> = entries.iter().map(|entry| &*entry.session_id).collect();
    let mut files = Vec::new();
    for root in roots {
        collect_files_with_extension(root, "jsonl", &mut files);
    }
    files
        .into_iter()
        .filter_map(|file| {
            // 文件名为 `<时间戳>_<sessionId>.jsonl`，与解析器的会话 ID 规则一致。
            let stem = file.file_stem()?.to_str()?;
            let session_id = stem.split_once('_').map_or(stem, |(_, id)| id);
            wanted
                .contains(session_id)
                .then(|| session_id.to_string())
                .zip(Some(file))
        })
        .filter_map(|(session_id, file)| Some((session_id, read_meta(&file)?)))
        .collect()
}

fn read_meta(file: &PathBuf) -> Option<SessionMeta> {
    let text = fs::read_to_string(file).ok()?;
    let mut meta = SessionMeta::default();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("session") {
            meta.project = value.get("cwd").and_then(Value::as_str).map(project_root);
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("user") if meta.title.is_none() => {
                meta.title = message.get("content").and_then(user_text);
            }
            Some("assistant") => {
                if let (Some(model), Some(provider)) = (
                    message.get("model").and_then(Value::as_str),
                    message.get("provider").and_then(Value::as_str),
                ) {
                    meta.model_providers
                        .entry(model.to_string())
                        .or_insert_with(|| provider.to_string());
                }
            }
            _ => {}
        }
    }
    Some(meta)
}
