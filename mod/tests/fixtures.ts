import type { SessionUsage } from 'claude-code'

import type { Bucket, ContextReport, HistoryRange, HistoryReport, Local, ModelSpend, Snapshot, Spend } from '../types'

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

const GROWTH = [20_000, 34_000, 52_000, 71_000, 96_000, 120_000, 151_000, 38_000, 41_000, 44_000, 46_000]

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
      points: GROWTH.map((prompt_tokens, index) => ({
        ts_ms: NOW_MS - (GROWTH.length - index) * MINUTE_MS,
        prompt_tokens,
        injected_tokens: 0,
        compacted: index === 7,
      })),
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

export const DAY_MS = 86_400_000
export const HOUR_MS = 3_600_000
export const OPUS = 'claude-opus-5-5'
export const SONNET = 'claude-sonnet-5-5'
export const HAIKU = 'claude-haiku-4-5'

type Spent = [model: string, usd: number]

export function spend(usd: number, calibrated = true): Spend {
  return {
    requests: Math.round(usd * 10),
    unpriced: 0,
    cost_usd: usd,
    reconciled_usd: 0,
    tokens: Math.round(usd * 100_000),
    pct_5h: calibrated ? usd * 2 : null,
  }
}

function summed(spends: Spend[]): Spend {
  return spends.reduce(
    (total, item) => ({
      requests: total.requests + item.requests,
      unpriced: total.unpriced + item.unpriced,
      cost_usd: total.cost_usd + item.cost_usd,
      reconciled_usd: total.reconciled_usd + item.reconciled_usd,
      tokens: total.tokens + item.tokens,
      pct_5h: total.pct_5h === null || item.pct_5h === null ? null : total.pct_5h + item.pct_5h,
    }),
    spend(0),
  )
}

export function bucket(startMs: number, label: string, spent: Spent[], hit: number, peak: number | null): Bucket {
  const models: ModelSpend[] = spent.map(([model, usd]) => ({ model, ...spend(usd) }))
  const total = summed(models)
  const prompt = total.tokens
  return {
    start_ms: startMs,
    label,
    ...total,
    models,
    prompt_tokens: prompt,
    cache_read_tokens: Math.round(prompt * hit),
    rebuilds: 0,
    rebuild_usd: 0,
    compactions: 0,
    peak_5h_pct: peak,
  }
}

const WEEK: [string, Spent[], number, number | null][] = [
  ['Oct 5', [[OPUS, 1.2], [SONNET, 0.3]], 0.82, 12],
  ['Oct 6', [], 0, null],
  ['Oct 7', [[OPUS, 2.4], [SONNET, 0.5], [HAIKU, 0.02]], 0.9, 48],
  ['Oct 8', [[OPUS, 0.8]], 0.76, 20],
  ['Oct 9', [[OPUS, 3.1], [SONNET, 1.1]], 0.93, 85],
  ['Oct 10', [[OPUS, 4], [SONNET, 0.4], [HAIKU, 0.05]], 0.95, 62],
  ['Oct 11', [[OPUS, 2.1]], 0.91, 30],
]

export const OCT_5_MS = Date.parse('2026-10-05T00:00:00+02:00')

export function historyReport(overrides: Partial<HistoryReport> = {}): HistoryReport {
  const buckets = WEEK.map(([label, spent, hit, peak], index) => bucket(OCT_5_MS + index * DAY_MS, label, spent, hit, peak))
  buckets[4] = { ...(buckets[4] as Bucket), rebuilds: 2, rebuild_usd: 0.61, compactions: 1 }
  const models: ModelSpend[] = [
    { model: OPUS, ...spend(13.6) },
    { model: SONNET, ...spend(2.3) },
    { model: HAIKU, ...spend(0.07) },
  ]
  const totals = summed(models)
  return {
    range: '7d',
    bucket: 'day',
    generated_at_ms: NOW_MS,
    since_ms: OCT_5_MS,
    until_ms: OCT_5_MS + 7 * DAY_MS,
    calibration: { calibrated: true, pairs: 31, needed: 20 },
    totals: {
      ...totals,
      sessions: 9,
      hit_ratio: 0.91,
      saved_usd: 21.4,
      rebuilds: 2,
      rebuild_usd: 0.61,
      idle_rebuilds: 1,
      compactions: 1,
      peak_5h_pct: 85,
    },
    buckets,
    models,
    projects: [
      { name: 'claude-plus', path: '/work/claude-plus', sessions: 5, ...spend(11.2) },
      { name: 'shop', path: '/work/shop', sessions: 3, ...spend(4.1) },
      { name: 'notes', path: '/work/notes', sessions: 1, ...spend(0.67) },
    ],
    sessions: [
      { id: 'a1b2c3d4-0000-0000-0000-000000000001', title: 'Plan the History view', project: 'claude-plus', last_ms: NOW_MS - 2 * HOUR_MS, ...spend(6.4) },
      { id: 'a1b2c3d4-0000-0000-0000-000000000002', title: null, project: 'shop', last_ms: NOW_MS - DAY_MS, ...spend(3.9) },
    ],
    ...overrides,
  }
}

export function dayReport(): HistoryReport {
  const start = Date.parse('2026-10-10T10:00:00Z')
  const buckets = Array.from({ length: 24 }, (_, index) => {
    const at = new Date(start + index * HOUR_MS)
    const label = `${String((at.getUTCHours() + 2) % 24).padStart(2, '0')}:00`
    const usd = index % 5 === 0 ? 0 : (index % 7) * 0.15
    return bucket(start + index * HOUR_MS, label, usd === 0 ? [] : [[OPUS, usd]], 0.9, usd === 0 ? null : usd * 10)
  })
  return historyReport({ range: '24h' as HistoryRange, bucket: 'hour', since_ms: start, until_ms: start + 24 * HOUR_MS, buckets })
}
