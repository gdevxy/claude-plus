import type { SessionUsage } from 'claude-code'

import type { ContextReport, Local, Snapshot } from '../types'

export const NOW_MS = Date.parse('2026-10-11T09:00:00Z')
export const MINUTE_MS = 60_000

export function snapshot(overrides: Partial<Snapshot['live']> = {}): Snapshot {
  return {
    session: 's1',
    known: true,
    generated_at_ms: NOW_MS,
    totals: {
      cost_usd: 4.18,
      reconciled_usd: 0,
      unpriced_requests: 0,
      requests: 42,
      subagents: 2,
      last_request_ms: NOW_MS - 2 * MINUTE_MS,
      tokens: { input: 12_000, output: 61_000, thinking: 0, cache_read: 1_900_000, cache_write_5m: 0, cache_write_1h: 128_000 },
    },
    live: {
      model: 'claude-opus-5-5',
      effort: 'high',
      cache: {
        state: 'warm',
        ttl: '1h',
        ttl_source: 'observed',
        last_touch_ms: NOW_MS - 7 * MINUTE_MS - 46_000,
        expires_at_ms: NOW_MS + 52 * MINUTE_MS + 14_000,
        hit_ratio: 0.93,
        saved_usd: 3.2,
        rebuilds: 1,
        rebuild_usd: 0.4,
        last_rebuild: { ts_ms: NOW_MS - 40 * MINUTE_MS, cause: 'idle', tokens: 51_000, extra_usd: 0.4 },
      },
      forecast: {
        model: 'claude-opus-5-5',
        samples: 10,
        prefix_tokens: 46_000,
        new_tokens: 2_030,
        output_tokens: 600,
        warm_usd: 0.03,
        cold_usd: 0.4,
        warm_pct_5h: 0.015,
        cold_pct_5h: 0.2,
        alternative: { model: 'claude-sonnet-5-5', first_usd: 0.2, next_usd: 0.02 },
      },
      limits: [
        {
          window: 'five_hour',
          used_pct: 54,
          resets_at_ms: NOW_MS + 2 * 60 * MINUTE_MS + 10 * MINUTE_MS,
          sampled_at_ms: NOW_MS - MINUTE_MS,
          reset_since_sample: false,
          pace: { pct_per_hour: 12.5, projected_pct: 81, full_at_ms: null },
          calibration: { family: 'opus', pairs: 23, needed: 20, pct_per_usd: 0.5 },
        },
        {
          window: 'seven_day',
          used_pct: 31,
          resets_at_ms: NOW_MS + 3 * 24 * 60 * MINUTE_MS,
          sampled_at_ms: NOW_MS - MINUTE_MS,
          reset_since_sample: false,
          pace: null,
          calibration: { family: 'opus', pairs: 0, needed: 20, pct_per_usd: null },
        },
      ],
      ...overrides,
    },
  }
}

export function local(overrides: Partial<Local> = {}): Local {
  return {
    session: 's1',
    model: 'claude-opus-5-5',
    effort: null,
    turns: 14,
    working: false,
    measure: {
      atMs: NOW_MS - MINUTE_MS,
      contextTokens: 46_000,
      contextWindow: 200_000,
      contextPercent: 23,
      rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAtMs: NOW_MS + 130 * MINUTE_MS }],
      costUsd: 4.18,
    },
    step: null,
    breakdown: null,
    ...overrides,
  }
}

export function usage(): SessionUsage {
  return {
    startedAt: NOW_MS - 60 * MINUTE_MS,
    context: {
      tokens: 46_000,
      window: 200_000,
      percent: 23,
      breakdown: {
        model: 'claude-opus-5-5',
        totalTokens: 46_000,
        maxTokens: 200_000,
        rawMaxTokens: 200_000,
        autocompactSource: 'auto',
        percentage: 23,
        gridRows: [[{ color: 'claude', isFilled: true, categoryName: 'Messages', tokens: 20_000, percentage: 10, squareFullness: 1 }]],
        isAutoCompactEnabled: true,
        autoCompactThreshold: 167_000,
        categories: [
          { name: 'System prompt', tokens: 3_100, color: 'promptBorder', isDeferred: false, kind: 'used' },
          { name: 'System tools', tokens: 14_200, color: 'inactive', isDeferred: false, kind: 'used' },
          { name: 'MCP tools', tokens: 4_100, color: 'permission', isDeferred: false, kind: 'used' },
          { name: 'MCP tools (deferred)', tokens: 8_400, color: 'inactive', isDeferred: true, kind: 'deferred' },
          { name: 'Memory files', tokens: 2_000, color: 'claude', isDeferred: false, kind: 'used' },
          { name: 'Skills', tokens: 2_600, color: 'warning', isDeferred: false, kind: 'used' },
          { name: 'Messages', tokens: 20_000, color: 'purple_FOR_SUBAGENTS_ONLY', isDeferred: false, kind: 'used' },
          { name: 'Free space', tokens: 121_000, color: 'promptBorder', isDeferred: false, kind: 'free' },
          { name: 'Autocompact buffer', tokens: 33_000, color: 'inactive', isDeferred: false, kind: 'buffer' },
        ],
        memoryFiles: [{ path: '/work/app/CLAUDE.md', type: 'Project', tokens: 1_600 }],
        mcpTools: [{ name: 'mcp__linear__search', serverName: 'linear', tokens: 4_100, isLoaded: true }],
        agents: [{ agentType: 'reviewer', source: 'userSettings', tokens: 120 }],
        skills: {
          totalSkills: 2,
          includedSkills: 2,
          tokens: 2_600,
          skillFrontmatter: [{ name: 'superpowers:brainstorming', source: 'plugin', pluginName: 'superpowers', tokens: 2_300 }],
        },
        apiUsage: { input_tokens: 3, output_tokens: 500, cache_read_input_tokens: 40_000, cache_creation_input_tokens: 6_000 },
      },
    },
    rateLimits: [],
    cost: { usd: 4.18 },
  }
}

export function contextReport(overrides: Partial<ContextReport> = {}): ContextReport {
  return {
    session: 's1',
    generated_at_ms: NOW_MS,
    project_dir: '/work/app',
    live: {
      ts_ms: NOW_MS - 2 * MINUTE_MS,
      model: 'claude-opus-5-5',
      total_tokens: 46_000,
      max_tokens: 200_000,
      percentage: 23,
      auto_compact_threshold: 167_000,
      categories: [
        { name: 'System prompt', tokens: 3_100, color: 'promptBorder', kind: 'used' },
        { name: 'System tools', tokens: 14_200, color: 'inactive', kind: 'used' },
        { name: 'MCP tools', tokens: 4_100, color: 'permission', kind: 'used' },
        { name: 'MCP tools (deferred)', tokens: 8_400, color: 'inactive', kind: 'deferred' },
        { name: 'Memory files', tokens: 2_000, color: 'claude', kind: 'used' },
        { name: 'Skills', tokens: 2_600, color: 'warning', kind: 'used' },
        { name: 'Messages', tokens: 20_000, color: 'purple_FOR_SUBAGENTS_ONLY', kind: 'used' },
        { name: 'Free space', tokens: 121_000, color: 'promptBorder', kind: 'free' },
        { name: 'Autocompact buffer', tokens: 33_000, color: 'inactive', kind: 'buffer' },
      ],
    },
    history: {
      first_request_ms: NOW_MS - 50 * MINUTE_MS,
      measured_tokens: 20_000,
      first_message_tokens: 100,
      scale: 1,
      initial: [
        { part: 'system', tokens: 16_900 },
        { part: 'memory', tokens: 1_000 },
        { part: 'skills', tokens: 2_000 },
      ],
      now: [
        { part: 'system', tokens: 16_900 },
        { part: 'memory', tokens: 1_500 },
        { part: 'skills', tokens: 2_000 },
        { part: 'messages', tokens: 25_600 },
      ],
      injected_later_tokens: 12_400,
      compactions: 1,
      points: [],
    },
    ratios: { prose: 3.5, code: 3, schema: 2.5, calibrated: false, pairs: 0 },
    items: [
      { kind: 'mcp_server', name: 'linear', detail: '2 tools', tokens: 4_100, estimated: false, uses_session: 1, uses_14d: 3 },
      { kind: 'skill', name: 'superpowers:brainstorming', detail: 'superpowers', tokens: 2_300, estimated: false, uses_session: 0, uses_14d: 0 },
      { kind: 'memory_file', name: 'CLAUDE.md', detail: 'Project', tokens: 1_600, estimated: false, uses_session: null, uses_14d: null },
      { kind: 'agent', name: 'reviewer', detail: null, tokens: 120, estimated: false, uses_session: null, uses_14d: null },
    ],
    dead_weight: [
      { kind: 'skill', name: 'superpowers:brainstorming', detail: 'superpowers', tokens: 2_300, estimated: false, uses_session: 0, uses_14d: 0 },
    ],
    tool_results: [
      { tool: 'Read', ts_ms: NOW_MS - 5 * MINUTE_MS, chars: 36_000, images: 0, tokens: 12_000 },
      { tool: 'mcp__linear__search', ts_ms: NOW_MS - 4 * MINUTE_MS, chars: 12_600, images: 0, tokens: 4_200 },
    ],
    config: {
      scanned_at_ms: NOW_MS - 10_000,
      groups: [
        { kind: 'claude_md', loading: 'always', count: 2, tokens: 900 },
        { kind: 'claude_md', loading: 'on_demand', count: 1, tokens: 2_000 },
        { kind: 'skill', loading: 'listing', count: 64, tokens: 8_000 },
        { kind: 'skill', loading: 'on_demand', count: 2, tokens: 5_000 },
        { kind: 'mcp_server', loading: 'unknown', count: 3, tokens: 0 },
        { kind: 'plugin', loading: 'always', count: 4, tokens: 0 },
        { kind: 'plugin', loading: 'off', count: 1, tokens: 0 },
      ],
    },
    ...overrides,
  }
}
