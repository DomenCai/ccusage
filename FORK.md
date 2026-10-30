# AgentLens fork

在上游 ccusage 基础上加了一个命令：`ccusage entries`，按次输出每条请求，供 AgentLens 自己聚合。另有一处计价差异，其余行为与上游一致：

- pi：Auto 模式下把记录里的 `cost.total: 0` 当作没有成本，改按模型价格估算（价格未知则输出 `null`）。pi 对没配价格的 provider（如自建代理）一律写 0，上游会照单显示 $0，掩盖真实用量。代价是真正免费的模型也会被按公开价格估算。

## 用法

```
ccusage entries --json -O [--since YYYYMMDD] [--until YYYYMMDD] [-z TZ] [--sources claude,codex,...]
```

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
      "inputTokens": 2,
      "outputTokens": 568,
      "cacheCreationTokens": 38876,
      "cacheReadTokens": 0,
      "totalTokens": 39446,
      "cost": 0.0080594
    }
  ],
  "detectedAgents": ["claude", "codex"]
}
```

- `cost` 为 `null` 表示价格未知，不是 0。
- `inputTokens` 不含缓存读写；`totalTokens` 还包含个别来源的额外 token（如推理 token 未拆分的情况）。
- Codex 没有项目信息，`project` 为空字符串；`sessionId` 是 rollout 文件的相对路径。
- `detectedAgents` 只在不带 `--since` 时完整：带 `--since` 时 Claude 会跳过窗口外的文件，窗口内无数据就不算检测到。

## 构建与发布

```
cd rust && cargo build --release -p ccusage --features fetch-litellm-pricing
```

推送 `agentlens-v*` 标签后，`.github/workflows/agentlens-release.yaml` 构建 `ccusage-darwin-arm64.tar.gz` 并发布到 release。
