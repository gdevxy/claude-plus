import { describe, expect, test } from 'claude-code/testing'

import type { AlertInput } from '../hooks/alerts'
import { EMPTY_ALERTED, alertsOf, ownAlerts, thresholdOf, wantsThreshold } from '../hooks/alerts'
import type { Alerted, LimitWindow, Local } from '../types'
import { MINUTE_MS, NOW_MS, local, snapshot } from './fixtures'

const OK = { status: 'ok', detail: null } as const
const KNOWN: Alerted = { ...EMPTY_ALERTED, session: 's1' }

function input(overrides: Partial<AlertInput> = {}): AlertInput {
  return { nowMs: NOW_MS, snapshot: snapshot(), local: local(), daemon: OK, known: KNOWN, ...overrides }
}

function expiring(inMs: number) {
  const base = snapshot()
  return snapshot({ cache: { ...base.live.cache, expires_at_ms: NOW_MS + inMs } })
}

function pacing(fullInMs: number | null, resetsInMs: number) {
  const base = snapshot()
  const five = base.live.limits[0] as LimitWindow
  const limit: LimitWindow = {
    ...five,
    resets_at_ms: NOW_MS + resetsInMs,
    pace: { pct_per_hour: 40, projected_pct: 120, full_at_ms: fullInMs === null ? null : NOW_MS + fullInMs },
  }
  return snapshot({ limits: [limit] })
}

function near(tokens: number, threshold: number | null): Local {
  const base = local()
  const measure = base.measure
  if (measure === null) throw new Error('no measure')
  const breakdown =
    threshold === null
      ? null
      : { ts_ms: NOW_MS, model: 'claude-opus-5-5', total_tokens: tokens, max_tokens: 200_000, percentage: 80, auto_compact_threshold: threshold, categories: [] }
  return local({ measure: { ...measure, contextTokens: tokens, contextPercent: Math.round(tokens / 2000), rateLimits: [] }, breakdown })
}

describe('alerts', () => {
  test('the cache warns once in its last minute, with what rewarming would cost', () => {
    const first = alertsOf(input({ snapshot: expiring(59_000) }))
    expect(first.alerts).toEqual([{ kind: 'cache', text: 'Cache goes cold in under a minute ✻ rewarming later costs ~$0.40' }])
    expect(first.next.cacheFor).toBe(NOW_MS + 59_000)
    expect(alertsOf(input({ nowMs: NOW_MS + 29_000, snapshot: expiring(59_000), known: first.next })).alerts).toEqual([])
    expect(alertsOf(input({ snapshot: expiring(61_000) })).alerts).toEqual([])
    expect(alertsOf(input({ snapshot: expiring(59_000), local: local({ working: true }) })).alerts).toEqual([])
    const renewed = alertsOf(input({ snapshot: expiring(40_000), known: { ...first.next, cacheFor: NOW_MS - 3 * MINUTE_MS } }))
    expect(renewed.alerts.map(alert => alert.kind)).toEqual(['cache'])
  })

  test('the cache warning leaves the price out when none is forecast', () => {
    const base = expiring(10_000)
    const quiet = { ...base, live: { ...base.live, forecast: null } }
    expect(alertsOf(input({ snapshot: quiet })).alerts).toEqual([{ kind: 'cache', text: 'Cache goes cold in under a minute ✻' }])
  })

  test('a pace that fills the 5h window before its reset warns once per window', () => {
    const first = alertsOf(input({ snapshot: pacing(42 * MINUTE_MS, 90 * MINUTE_MS), local: local({ measure: null }) }))
    expect(first.alerts).toEqual([{ kind: 'pace', text: 'At this pace the 5h limit runs out in 42m, before its reset in 1h 30m' }])
    const drifted = pacing(40 * MINUTE_MS, 90 * MINUTE_MS + 20_000)
    expect(alertsOf(input({ snapshot: drifted, local: local({ measure: null }), known: first.next })).alerts).toEqual([])
    const nextWindow = pacing(40 * MINUTE_MS, 5 * 60 * MINUTE_MS)
    expect(alertsOf(input({ snapshot: nextWindow, local: local({ measure: null }), known: first.next })).alerts.map(alert => alert.kind)).toEqual(['pace'])
    expect(alertsOf(input({ snapshot: pacing(null, 90 * MINUTE_MS), local: local({ measure: null }) })).alerts).toEqual([])
  })

  test('context warns at 90% of the auto-compact threshold and again only after dropping under 80%', () => {
    const first = alertsOf(input({ local: near(151_000, 167_000) }))
    expect(first.alerts).toEqual([{ kind: 'context', text: 'Context is close to auto-compact, 151k of 167k ✻ /compact now to choose what stays' }])
    const still = alertsOf(input({ local: near(160_000, 167_000), known: first.next }))
    expect([still.alerts, still.next.contextFired]).toEqual([[], true])
    const compacted = alertsOf(input({ local: near(40_000, 167_000), known: still.next }))
    expect(compacted.next.contextFired).toBe(false)
    expect(alertsOf(input({ local: near(155_000, 167_000), known: compacted.next })).alerts.map(alert => alert.kind)).toEqual(['context'])
    expect(alertsOf(input({ local: near(190_000, null) })).alerts).toEqual([])
    expect(alertsOf(input({ local: near(155_000, null), known: { ...KNOWN, threshold: 167_000 } })).alerts.map(alert => alert.kind)).toEqual(['context'])
  })

  test('the threshold is asked for once a session passes 60% without a breakdown', () => {
    expect(wantsThreshold(near(130_000, null), KNOWN)).toBe(true)
    expect(wantsThreshold(near(100_000, null), KNOWN)).toBe(false)
    expect(wantsThreshold(near(130_000, 167_000), KNOWN)).toBe(false)
    expect(wantsThreshold(near(130_000, null), { ...KNOWN, thresholdAsked: true })).toBe(false)
    expect(wantsThreshold(near(130_000, null), { ...KNOWN, session: 's0', thresholdAsked: true })).toBe(true)
    expect(thresholdOf(near(1, 167_000), { ...KNOWN, threshold: 150_000 })).toBe(167_000)
  })

  test('a new session starts with nothing alerted', () => {
    const before: Alerted = { session: 's0', cacheFor: 1, paceFor: 2, contextFired: true, threshold: 3, thresholdAsked: true }
    expect(ownAlerts(before, 's1')).toEqual({ ...EMPTY_ALERTED, session: 's1' })
    expect(ownAlerts(before, 's0')).toBe(before)
    expect(alertsOf(input({ snapshot: expiring(59_000), known: { ...before, cacheFor: NOW_MS + 59_000 } })).alerts.map(alert => alert.kind)).toEqual(['cache'])
  })
})
