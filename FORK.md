# AgentLens fork

在上游 ccusage 基础上加了一个命令：`ccusage entries`，按次输出每条请求，供 AgentLens 自己聚合。另有两处计价差异，其余行为与上游一致：

- pi：Auto 模式下把记录里的 `cost.total: 0` 当作没有成本，改按模型价格估算（价格未知则输出 `null`）。pi 对没配价格的 provider（如自建代理）一律写 0，上游会照单显示 $0，掩盖真实用量。代价是真正免费的模型也会被按公开价格估算。
- 用户价格覆盖（配置文件的 `pricingOverrides`）优先于来源自带的成本：上游 Auto 模式下只要记录里带成本（Claude `costUSD`、Grok ticks、OpenCode、Kilo、Hermes 等）就直接用，手动价不生效。现在模型名或其别名有覆盖时，一律按覆盖价重算。入口是 `ccusage-core` 的 `recorded_cost_for_auto`，Grok、Kilo、OpenCode、Hermes 的自有分支也改为调用它。

## 用法

```
ccusage entries --json -O [--since YYYYMMDD] [--until YYYYMMDD] [-z TZ] [--sources claude,codex,...] [--config PATH]
```

- 手动价格和别名沿用上游机制：`--config` 指向含 `defaults.pricingOverrides` 的配置文件（可以是 `/dev/stdin`）；别名用环境变量 `CCUSAGE_MODEL_ALIASES='{"原名":"目标名"}'`，输出的 `model` 变为目标名，原名自己查不到价格时借用目标名的价格。

- 输出总是紧凑 JSON（`--json` 可省略），警告写到 stderr。
- `--sources` 只加载列出的内置来源（名字同 `BUILT_IN_AGENT_NAMES`），不写则加载全部。
- 每个来源走与 `ccusage daily` 相同的适配器入口，去重和计价一致；按 agent 汇总后与 `daily --by-agent` 的成本、tokens 相同。

## 输出格式

```json
{
  "entries": [
    {
      "agent": "claude",
      "timestamp": "2026-10-29T22:41:18.648Z",
      "sessionId": "1d4e913b-…",
      "project": "-Users-me-Claude-Scheduled",
      "model": "claude-haiku-5-5",
      "provider": null,
      "inputTokens": 2,
      "outputTokens": 568,
      "cacheCreationTokens": 38876,
      "cacheReadTokens": 0,
      "totalTokens": 39446,
      "cost": 0.0080594
    }
  ],
  "sessions": {
    "claude|1d4e913b-…": { "project": "/Users/me/Claude/Scheduled", "title": "每日摘要" }
  },
  "pricing": {
    "claude-haiku-5-5": {
      "inputCostPerToken": 1e-7,
      "outputCostPerToken": 5e-7,
      "cacheCreationInputTokenCost": 1.25e-7,
      "cacheReadInputTokenCost": 1e-8
    }
  },
  "detectedAgents": ["claude", "codex"]
}
```

- `cost` 为 `null` 表示价格未知，不是 0。
- `inputTokens` 不含缓存读写；`totalTokens` 还包含个别来源的额外 token（如推理 token 未拆分的情况）。
- `project` 是来源原始的项目标识（Claude 为编码后的目录名，Codex 为空字符串），只作兜底；真实目录看 `sessions`。Codex 的 `sessionId` 是 rollout 文件的相对路径。
- `model` 去掉了 pi 系来源的 `[store] ` 前缀，同一模型在不同来源下同名。
- `provider` 是 API 渠道：pi 取每条消息的 `provider`，Codex 取会话的 `model_provider`，其他来源为 `null`。
- `sessions` 键为 `<agent>|<sessionId>`，只含窗口内有记录的会话，目前覆盖 claude、codex、pi、kimi、grok、antigravity（各适配器的 `meta.rs`）：
  - `project`：会话启动时的工作目录；git worktree（含 `.claude/worktrees/`）归到主仓库根目录。
  - `title`：来源自带的会话标题（Claude `custom-title`、Codex `session_index.jsonl`、Kimi `state.json`、Grok `generated_title`、Antigravity 摘要库），没有时用截断的首条用户消息；可能为 `null`。
- `pricing` 是输出中每个模型在统一价格表里查到的基础单价（美元/token，已含用户覆盖），键同 `entries[].model`，查不到的模型不出现。只供展示：各来源计价时还会试自己的候选名、阶梯价和来源自带成本，所以单价存在不代表每条 `cost` 都由它算出，单价缺失也不代表 `cost` 为 `null`。
- `detectedAgents` 只在不带 `--since` 时完整：带 `--since` 时 Claude 会跳过窗口外的文件，窗口内无数据就不算检测到。

## 构建与发布

```
cd rust && cargo build --release -p ccusage --features fetch-litellm-pricing
```

推送 `agentlens-v*` 标签后，`.github/workflows/agentlens-release.yaml` 构建 `ccusage-darwin-arm64.tar.gz` 并发布到 release。
