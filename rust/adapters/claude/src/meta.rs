//! AgentLens fork：按会话补读真实工作目录和标题。

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use ccusage_adapter_common::{
    read_files_parallel,
    session_meta::{SessionMeta, clip_title, project_root, user_text},
};
use ccusage_core::LoadedEntry;
use serde_json::Value;

use crate::paths::claude_paths;

/// 键为 sessionId。每行都带 `cwd`，取第一条；标题取最后一条 `custom-title`，没有则用首条用户消息。
/// 只有子代理记录的会话（主文件已删除）从 `<session>/subagents/` 里取。
pub fn session_meta(entries: &[LoadedEntry]) -> HashMap<String, SessionMeta> {
    let roots = claude_paths().unwrap_or_default();
    let sessions: HashSet<(&str, &str)> = entries
        .iter()
        .map(|entry| (&*entry.project, &*entry.session_id))
        .collect();
    let mut ids = Vec::new();
    let mut files = Vec::new();
    for (project, session_id) in sessions {
        if let Some(file) = session_file(&roots, project, session_id) {
            ids.push(session_id);
            files.push(file);
        }
    }
    let metas = read_files_parallel(&files, false, read_meta);
    ids.into_iter()
        .zip(metas)
        .filter_map(|(id, meta)| Some((id.to_string(), meta?)))
        .collect()
}

fn session_file(roots: &[PathBuf], project: &str, session_id: &str) -> Option<PathBuf> {
    roots.iter().find_map(|root| {
        let dir = root.join("projects").join(project);
        let main = dir.join(format!("{session_id}.jsonl"));
        if main.is_file() {
            return Some(main);
        }
        fs::read_dir(dir.join(session_id).join("subagents"))
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.path()))
            .find(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
    })
}

/// 用子串定位所需的几行再解析，避免逐行解析大段工具输出。
fn read_meta(file: &Path) -> Option<SessionMeta> {
    let text = fs::read_to_string(file).ok()?;
    let parse = |line: &str| serde_json::from_str::<Value>(line).ok();
    let project = text
        .find("\"cwd\":\"")
        .and_then(|at| parse(line_at(&text, at)))
        .and_then(|value| value.get("cwd")?.as_str().map(project_root));
    let title = text
        .rfind("\"type\":\"custom-title\"")
        .and_then(|at| parse(line_at(&text, at)))
        .and_then(|value| value.get("customTitle")?.as_str().and_then(clip_title))
        .or_else(|| {
            text.lines()
                .filter(|line| line.contains("\"type\":\"user\""))
                .filter_map(parse)
                .filter(|value| value.get("isMeta").and_then(Value::as_bool) != Some(true))
                .find_map(|value| user_text(value.pointer("/message/content")?))
        });
    (project.is_some() || title.is_some()).then(|| SessionMeta {
        project,
        title,
        ..SessionMeta::default()
    })
}

fn line_at(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |index| index + 1);
    let end = text[at..].find('\n').map_or(text.len(), |index| at + index);
    &text[start..end]
}
