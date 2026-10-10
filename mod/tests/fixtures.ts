import type { Local, Snapshot } from '../types'

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
    ...overrides,
  }
}
