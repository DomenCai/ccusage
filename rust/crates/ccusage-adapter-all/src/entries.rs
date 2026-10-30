//! `ccusage entries`: one JSON row per request, for tools that aggregate usage
//! themselves. Every source loads through the same adapter entry points the
//! unified reports use, so deduplication and pricing match `ccusage daily`.

use std::{
    collections::BTreeMap,
    io::{BufWriter, Write},
    thread,
};

use serde_json::{Value, json};

use crate::{
    BUILT_IN_AGENT_NAMES, LoadedEntry, PricingMap, Result,
    adapter::{
        amp, antigravity, claude, codebuff, codex, copilot, droid, gemini, goose, grok, hermes,
        kilo, kimi, openclaw, opencode, pi, qwen, zcode,
    },
    cli::{AgentReportKind, CodexSpeed, CostMode, EntriesArgs, SharedArgs},
    filter_loaded_entries_by_date, format_rfc3339_millis, json_float,
};

use super::loader::{load_pricing, resolve_named_pi_store_paths, shared_by_source};

struct SourceEntries {
    agent: &'static str,
    rows: Vec<Value>,
    detected: bool,
    warnings: Vec<String>,
}

pub fn run(args: EntriesArgs) -> Result<()> {
    let shared = args.shared;
    let wanted = |agent: &str| {
        args.sources
            .as_ref()
            .is_none_or(|sources| sources.iter().any(|name| name == agent))
    };
    let source_shared = shared_by_source(&BTreeMap::new(), &shared);
    let (pricing, unified_pricing) = load_pricing(&shared, &source_shared, true);
    let unified_pricing = unified_pricing.expect("unified pricing is requested");
    let named_pi_stores = if wanted("pi") {
        resolve_named_pi_store_paths(&shared.pi_stores, None)?
    } else {
        Vec::new()
    };

    let loaded = thread::scope(|scope| {
        let mut handles = Vec::new();
        for agent in BUILT_IN_AGENT_NAMES.iter().copied().filter(|a| wanted(a)) {
            let shared = &source_shared[agent];
            let pricing = pricing.get(agent).map(AsRef::as_ref);
            handles.push(scope.spawn(move || load_source(agent, shared, pricing)));
        }
        for store in named_pi_stores {
            let shared = &source_shared["pi"];
            let pricing = unified_pricing.as_ref();
            let agent: &'static str = Box::leak(store.name.into_boxed_str());
            handles.push(scope.spawn(move || {
                let entries =
                    pi::load_entries_for_store_paths(shared, store.paths, agent, Some(pricing))?;
                Ok(loaded_entries_source(agent, shared, entries, false))
            }));
        }
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(crate::cli_error("entries loader panicked")))
            })
            .collect::<Result<Vec<_>>>()
    })?;

    let mut rows = Vec::new();
    let mut detected = Vec::new();
    for source in loaded {
        source.warnings.iter().for_each(|w| eprintln!("{w}"));
        if source.detected {
            detected.push(source.agent);
        }
        rows.extend(source.rows);
    }
    let output = json!({ "entries": rows, "detectedAgents": detected });
    // Compact on purpose: remote machines stream this over SSH every poll.
    let stdout = std::io::stdout();
    let mut stdout = BufWriter::new(stdout.lock());
    serde_json::to_writer(&mut stdout, &output)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn load_source(
    agent: &'static str,
    shared: &SharedArgs,
    pricing: Option<&PricingMap>,
) -> Result<SourceEntries> {
    // Sources outside this list price themselves and have no shared map.
    let priced = || pricing.expect("source has a shared pricing map");
    let entries = match agent {
        "claude" => {
            let entries = if shared.since.is_some() {
                claude::load_entries_since(shared, None)?
            } else {
                claude::load_entries(shared, None)?
            };
            return Ok(loaded_entries_source(agent, shared, entries, false));
        }
        "codex" => return load_codex(shared, priced()),
        "opencode" => {
            let entries = opencode::load_entries(shared, AgentReportKind::Daily)?;
            return Ok(loaded_entries_source(
                agent,
                shared,
                entries,
                opencode::has_data(),
            ));
        }
        "qwen" => {
            let entries = qwen::load_entries(shared)?;
            return Ok(loaded_entries_source(
                agent,
                shared,
                entries,
                qwen::has_data(),
            ));
        }
        "grok" => {
            let entries = grok::load_entries(shared, priced())?;
            return Ok(loaded_entries_source(
                agent,
                shared,
                entries,
                grok::has_data(),
            ));
        }
        "antigravity" => {
            let (entries, warnings) = antigravity::load_entries_with_warnings(shared, priced())?;
            let mut source = loaded_entries_source(agent, shared, entries, false);
            source.warnings = warnings;
            return Ok(source);
        }
        "pi" => pi::load_entries(shared, None, Some(priced()))?,
        "openclaw" => openclaw::load_entries(shared, None, Some(priced()))?,
        "amp" => amp::load_entries(shared, priced())?,
        "droid" => droid::load_entries(shared, priced())?,
        "codebuff" => codebuff::load_entries(shared, priced())?,
        "hermes" => hermes::load_entries(shared, priced())?,
        "goose" => goose::load_entries(shared, priced())?,
        "kilo" => kilo::load_entries(shared, priced())?,
        "copilot" => copilot::load_entries(shared, priced())?,
        "gemini" => gemini::load_entries(shared, priced())?,
        "kimi" => kimi::load_entries(shared, priced())?,
        "zcode" => zcode::load_entries(shared, priced())?,
        other => unreachable!("unknown built-in agent {other}"),
    };
    Ok(loaded_entries_source(agent, shared, entries, false))
}

fn loaded_entries_source(
    agent: &'static str,
    shared: &SharedArgs,
    mut entries: Vec<LoadedEntry>,
    has_data: bool,
) -> SourceEntries {
    let detected = has_data || !entries.is_empty();
    filter_loaded_entries_by_date(&mut entries, shared);
    let rows = entries
        .iter()
        .filter_map(|entry| loaded_entry_json(agent, entry))
        .collect();
    SourceEntries {
        agent,
        rows,
        detected,
        warnings: Vec::new(),
    }
}

fn loaded_entry_json(agent: &str, entry: &LoadedEntry) -> Option<Value> {
    let usage = entry.data.message.usage;
    let cache_creation = usage.cache_creation_token_count();
    let total = usage
        .input_tokens
        .saturating_add(usage.output_tokens)
        .saturating_add(cache_creation)
        .saturating_add(usage.cache_read_input_tokens)
        .saturating_add(entry.extra_total_tokens);
    // Same rule as the unified rows: cost-only entries still bill.
    if total == 0 && entry.cost == 0.0 {
        return None;
    }
    let model = entry
        .model
        .as_deref()
        .or(entry.data.message.model.as_deref())
        .map(|model| crate::model_aliases::resolve_model_name(model).into_owned());
    let cost = if entry.missing_pricing_model.is_some() {
        Value::Null
    } else {
        json_float(entry.cost)
    };
    Some(json!({
        "agent": agent,
        "timestamp": format_rfc3339_millis(entry.timestamp),
        "sessionId": &*entry.session_id,
        "project": &*entry.project,
        "model": model,
        "inputTokens": usage.input_tokens,
        "outputTokens": usage.output_tokens,
        "cacheCreationTokens": cache_creation,
        "cacheReadTokens": usage.cache_read_input_tokens,
        "totalTokens": total,
        "cost": cost,
    }))
}

/// Codex events carry cumulative counters and replayed compaction copies, which
/// `aggregate_events` resolves. Grouping by a per-event key keeps that logic and
/// yields one group per surviving request.
fn load_codex(shared: &SharedArgs, pricing: &PricingMap) -> Result<SourceEntries> {
    let (mut events, detected) = codex::load_codex_events_with_detection(shared)?;
    codex::filter_events_by_date(&mut events, shared)?;
    let session_ids: Vec<String> = events
        .iter_mut()
        .enumerate()
        .map(|(index, event)| std::mem::replace(&mut event.session_id, index.to_string()))
        .collect();
    let groups = codex::aggregate_events(
        &events,
        AgentReportKind::Session,
        shared.timezone.as_deref(),
    )?;
    let speed = codex::resolve_codex_speed(CodexSpeed::Auto);
    let mut rows = Vec::with_capacity(groups.len());
    for (key, group) in &groups {
        let index: usize = key.parse().expect("codex group key is an event index");
        let event = &events[index];
        let Some((model, usage)) = group.models.iter().next() else {
            continue;
        };
        let missing = shared.mode != CostMode::Display
            && codex::codex_model_missing_pricing(model, usage, pricing);
        let cost = if missing {
            Value::Null
        } else {
            json_float(codex::calculate_codex_model_cost(
                model, usage, pricing, speed,
            ))
        };
        let timestamp = crate::parse_ts_timestamp(&event.timestamp)
            .map_or_else(|| event.timestamp.clone(), format_rfc3339_millis);
        rows.push(json!({
            "agent": "codex",
            "timestamp": timestamp,
            "sessionId": &session_ids[index],
            "project": "",
            "model": model,
            "inputTokens": codex::non_cached_input_tokens(
                group.input_tokens,
                group.cached_input_tokens,
                group.cache_creation_tokens,
            ),
            "outputTokens": group.output_tokens,
            "cacheCreationTokens": group.cache_creation_tokens,
            "cacheReadTokens": group.cached_input_tokens,
            "totalTokens": group.total_tokens,
            "cost": cost,
        }));
    }
    Ok(SourceEntries {
        agent: "codex",
        rows,
        detected,
        warnings: Vec::new(),
    })
}
