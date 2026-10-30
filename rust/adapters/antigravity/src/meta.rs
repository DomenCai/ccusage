//! AgentLens fork：从 `conversation_summaries.db` 补读会话的工作区和标题。

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use ccusage_adapter_common::session_meta::{SessionMeta, clip_title, project_root};
use ccusage_core::LoadedEntry;
use serde_json::Value;

use crate::paths::conversation_db_paths;

/// 键为 sessionId（conversation id）。摘要库在数据根目录，与 `conversations/` 同级；
/// `workspace_uris` 是 `file://` URI 数组，取第一个；标题为空时用 `preview`（首条消息）。
pub fn session_meta(entries: &[LoadedEntry]) -> HashMap<String, SessionMeta> {
    let wanted: HashSet<&str> = entries.iter().map(|entry| &*entry.session_id).collect();
    let summary_dbs: HashSet<PathBuf> = conversation_db_paths()
        .unwrap_or_default()
        .iter()
        .filter_map(|path| {
            let dir = path.parent()?;
            let root = if dir.ends_with("conversations") {
                dir.parent()?
            } else {
                dir
            };
            Some(root.join("conversation_summaries.db"))
        })
        .filter(|path| path.is_file())
        .collect();
    let mut result = HashMap::new();
    for db in summary_dbs {
        let Ok(connection) =
            sqlite::Connection::open_with_flags(&db, sqlite::OpenFlags::new().with_read_only())
        else {
            continue;
        };
        let Ok(mut statement) = connection.prepare(
            "SELECT conversation_id, title, preview, workspace_uris FROM conversation_summaries",
        ) else {
            continue;
        };
        while let Ok(sqlite::State::Row) = statement.next() {
            let text = |index: usize| statement.read::<String, _>(index).unwrap_or_default();
            let id = text(0);
            if !wanted.contains(id.as_str()) {
                continue;
            }
            let title = clip_title(&text(1)).or_else(|| clip_title(&text(2)));
            let project = serde_json::from_str::<Vec<Value>>(&text(3))
                .ok()
                .and_then(|uris| uris.first()?.as_str().map(str::to_string))
                .and_then(|uri| uri.strip_prefix("file://").map(percent_decode))
                .map(|path| project_root(&path));
            result.insert(
                id,
                SessionMeta {
                    project,
                    title,
                    ..SessionMeta::default()
                },
            );
        }
    }
    result
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = value
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}
