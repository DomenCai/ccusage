//! AgentLens fork：按会话补读真实工作目录、标题和 `model_provider`。

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::BufRead,
};

use ccusage_adapter_common::{
    read_files_parallel,
    session_meta::{SessionMeta, clip_title, project_root, user_text},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{paths::codex_home_paths, reader::open_rollout_reader};

/// 键为 sessionId（rollout 相对 `sessions/` 或 `archived_sessions/` 的路径，不含扩展名）。
/// rollout 首行 `session_meta` 带 `cwd` 和 `model_provider`；标题来自 `session_index.jsonl`
/// 的 `thread_name`，没有则用首条用户消息。
pub fn session_meta<'a>(
    session_ids: impl IntoIterator<Item = &'a str>,
) -> HashMap<String, SessionMeta> {
    let homes = codex_home_paths().unwrap_or_default();
    let thread_names = thread_names(&homes);
    let mut ids = Vec::new();
    let mut files = Vec::new();
    for session_id in session_ids.into_iter().collect::<HashSet<_>>() {
        // 压缩与否由 `open_rollout_reader` 处理，这里只需确认任一编码存在。
        let file = homes.iter().find_map(|home| {
            ["sessions", "archived_sessions"].iter().find_map(|dir| {
                let path = home.join(dir).join(format!("{session_id}.jsonl"));
                let exists = path.is_file() || path.with_extension("jsonl.zst").is_file();
                exists.then_some(path)
            })
        });
        if let Some(file) = file {
            ids.push(session_id);
            files.push(file);
        }
    }
    let metas = read_files_parallel(&files, false, |path| {
        let (reader, _) = open_rollout_reader(path).ok()?;
        Some(read_meta(reader, &thread_names))
    });
    ids.into_iter()
        .zip(metas)
        .filter_map(|(id, meta)| Some((id.to_string(), meta?)))
        .collect()
}

/// 只反序列化用到的字段：`session_meta` 行带着几十 KB 的 instructions，整行转 `Value` 很慢。
#[derive(Deserialize)]
struct MetaLine {
    payload: MetaPayload,
}

#[derive(Deserialize)]
struct MetaPayload {
    id: Option<String>,
    cwd: Option<String>,
    model_provider: Option<String>,
}

/// 首条用户消息之后的内容与标题无关；限制扫描行数，避免没有用户消息的会话被读完整个文件。
const PROMPT_SCAN_LINES: usize = 200;

fn read_meta(reader: Box<dyn BufRead>, thread_names: &HashMap<String, String>) -> SessionMeta {
    let mut meta = SessionMeta::default();
    for line in reader.lines().map_while(Result::ok).take(PROMPT_SCAN_LINES) {
        if meta.project.is_none() && line.contains("\"type\":\"session_meta\"") {
            let Ok(MetaLine { payload }) = serde_json::from_str::<MetaLine>(&line) else {
                continue;
            };
            meta.project = payload.cwd.as_deref().map(project_root);
            meta.provider = payload.model_provider;
            meta.title = payload
                .id
                .and_then(|id| thread_names.get(&id))
                .and_then(|name| clip_title(name));
            if meta.title.is_some() {
                break;
            }
            continue;
        }
        let is_event_prompt = line.contains("\"type\":\"user_message\"");
        if !(is_event_prompt || line.contains("\"role\":\"user\"")) {
            continue;
        }
        let Some(payload) = serde_json::from_str::<Value>(&line)
            .ok()
            .and_then(|mut value| value.get_mut("payload").map(Value::take))
        else {
            continue;
        };
        // 外部驱动的会话没有 `user_message` 事件，首条消息只在 `response_item` 里，
        // 且前面有一条注入的 AGENTS.md。
        let prompt = if is_event_prompt {
            payload
                .get("message")
                .and_then(Value::as_str)
                .and_then(clip_title)
        } else {
            payload
                .get("content")
                .filter(|_| payload.get("role").and_then(Value::as_str) == Some("user"))
                .and_then(user_text)
                .filter(|text| !text.starts_with("# AGENTS.md instructions"))
        };
        if let Some(title) = prompt {
            meta.title = Some(title);
            break;
        }
    }
    meta
}

/// `session_index.jsonl` 只追加，同一线程改名后后面的记录覆盖前面的。
fn thread_names(homes: &[std::path::PathBuf]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for home in homes {
        let Ok(text) = fs::read_to_string(home.join("session_index.jsonl")) else {
            continue;
        };
        for line in text.lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if let (Some(id), Some(name)) = (
                value.get("id").and_then(Value::as_str),
                value.get("thread_name").and_then(Value::as_str),
            ) {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }
    names
}
