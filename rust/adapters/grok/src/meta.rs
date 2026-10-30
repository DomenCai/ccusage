//! AgentLens fork：按会话补读标题，并把已解析出的工作目录归到主项目。

use std::{collections::HashMap, fs};

use ccusage_adapter_common::session_meta::{SessionMeta, clip_title, project_root};
use ccusage_core::LoadedEntry;
use serde_json::Value;

use crate::paths::discover_session_files;

/// 键为 sessionId。解析器已把 `summary.json` 的 `info.cwd` 放进 `project_path`；
/// 标题只有部分会话的 `generated_title` 有。
pub fn session_meta(entries: &[LoadedEntry]) -> HashMap<String, SessionMeta> {
    let mut titles = HashMap::new();
    for files in discover_session_files().unwrap_or_default() {
        let Some(summary) = files
            .summary
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        else {
            continue;
        };
        let id = summary
            .pointer("/info/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                let dir = files.updates.parent()?.file_name()?.to_str()?;
                Some(dir.to_string())
            });
        let title = summary
            .get("generated_title")
            .and_then(Value::as_str)
            .and_then(clip_title);
        if let (Some(id), Some(title)) = (id, title) {
            titles.insert(id, title);
        }
    }
    let mut result = HashMap::new();
    for entry in entries {
        let session_id = &*entry.session_id;
        if result.contains_key(session_id) {
            continue;
        }
        let project =
            (entry.project_path.starts_with('/')).then(|| project_root(&entry.project_path));
        let meta = SessionMeta {
            project,
            title: titles.get(session_id).cloned(),
            ..SessionMeta::default()
        };
        result.insert(session_id.to_string(), meta);
    }
    result
}
