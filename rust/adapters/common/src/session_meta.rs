//! AgentLens fork：`ccusage entries` 输出的会话元数据（真实项目路径、标题、provider）。
//! 各适配器在自己的 `meta.rs` 里按会话 ID 补读这些信息，这里只放共用的类型和规则。

use std::{collections::HashMap, fs, path::Path};

use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct SessionMeta {
    /// 会话启动时的工作目录，已按 [`project_root`] 归到主项目。
    pub project: Option<String>,
    pub title: Option<String>,
    /// 会话级 provider（如 Codex 的 `model_provider`）。
    pub provider: Option<String>,
    /// 按原始模型名区分的 provider（如 Pi 每条消息自带的 `provider`）。
    pub model_providers: HashMap<String, String>,
}

impl SessionMeta {
    pub fn provider_for(&self, model: &str) -> Option<&str> {
        self.model_providers
            .get(model)
            .or(self.provider.as_ref())
            .map(String::as_str)
    }
}

/// 把工作目录归到主项目：git worktree 归到主仓库根目录，其余原样返回。
pub fn project_root(cwd: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    // Claude Code 桌面版把 worktree 建在 `<repo>/.claude/worktrees/<name>`，删掉后也能识别。
    if let Some((repo, _)) = cwd.split_once("/.claude/worktrees/") {
        return repo.to_string();
    }
    // 其他工具的 worktree：根目录的 `.git` 是文件，内容为 `gitdir: <main>/.git/worktrees/<name>`。
    for dir in Path::new(cwd).ancestors() {
        let git = dir.join(".git");
        if git.is_dir() {
            break;
        }
        if git.is_file() {
            if let Some(main) = fs::read_to_string(&git).ok().and_then(|text| {
                let gitdir = text.trim().strip_prefix("gitdir:")?.trim().to_string();
                let (main, _) = gitdir.split_once("/.git/worktrees/")?;
                Some(main.to_string())
            }) {
                return main;
            }
            break;
        }
    }
    cwd.to_string()
}

/// 标题统一成单行并截断，避免把整段首条消息塞进输出。
pub fn clip_title(text: &str) -> Option<String> {
    const MAX_CHARS: usize = 80;
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.chars().count() <= MAX_CHARS {
        return Some(line);
    }
    let mut clipped: String = line.chars().take(MAX_CHARS).collect();
    clipped.push('…');
    Some(clipped)
}

/// 从消息 `content` 取用户输入的文字：字符串或 `[{type:"text"|"input_text", text}]` 数组。
/// 以 `<` 开头的是工具注入的命令/提示标签，不算用户输入。
pub fn user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => text.as_str(),
        Value::Array(blocks) => blocks.iter().find_map(|block| {
            matches!(block.get("type")?.as_str()?, "text" | "input_text")
                .then(|| block.get("text")?.as_str())
                .flatten()
        })?,
        _ => return None,
    };
    let text = text.trim();
    if text.starts_with('<') {
        return None;
    }
    clip_title(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_claude_worktree_to_repo() {
        assert_eq!(
            project_root("/Users/me/repo/.claude/worktrees/fix-bug"),
            "/Users/me/repo"
        );
    }

    #[test]
    fn resolves_git_worktree_file_to_main_repo() {
        let dir = std::env::temp_dir().join(format!("ccusage-meta-{}", std::process::id()));
        let worktree = dir.join("wt/game");
        fs::create_dir_all(worktree.join("src")).unwrap();
        fs::write(
            worktree.join(".git"),
            "gitdir: /Users/me/game/.git/worktrees/game\n",
        )
        .unwrap();
        assert_eq!(
            project_root(worktree.join("src").to_str().unwrap()),
            "/Users/me/game"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn keeps_plain_directories() {
        assert_eq!(project_root("/tmp/not-a-repo/"), "/tmp/not-a-repo");
    }

    #[test]
    fn skips_injected_tags_and_clips_long_text() {
        assert_eq!(
            user_text(&Value::from("<command-name>/x</command-name>")),
            None
        );
        let long = "字".repeat(100);
        assert_eq!(user_text(&Value::from(long)).unwrap().chars().count(), 81);
    }
}
