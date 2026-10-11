import { describe, expect, test } from 'claude-code/testing'

import type { AlertInput } from '../hooks/alerts'
import { EMPTY_ALERTED, alertsOf, merged, ownAlerts, thresholdOf, wantsThreshold } from '../hooks/alerts'
import type { Alerted, LimitWindow, Local } from '../types'
import { DAY_MS, MINUTE_MS, NOW_MS, local, snapshot } from './fixtures'

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

const PERIOD_END_MS = Date.parse('2026-11-01T00:00:00Z')

function spendWindow(used: number, window = 'spend_limit', resetsAtMs = PERIOD_END_MS): LimitWindow {
  return {
    window,
    used_pct: used,
    resets_at_ms: resetsAtMs,
    sampled_at_ms: NOW_MS - MINUTE_MS,
    reset_since_sample: false,
    pace: { pct_per_hour: 0.01, projected_pct: 60, full_at_ms: null },
    calibration: { family: 'opus', pairs: 0, needed: 20, pct_per_usd: null },
  }
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

function cappedAt(tokens: number, breakdownModel: string, threshold: number | null): Local {
  const measure = { ...local().measure!, contextTokens: tokens, contextWindow: 1_000_000, contextPercent: Math.round(tokens / 10_000), rateLimits: [] }
  const breakdown = { ts_ms: NOW_MS, model: breakdownModel, total_tokens: tokens, max_tokens: 400_000, percentage: 0, auto_compact_threshold: threshold, categories: [] }
  return local({ model: 'claude-sonnet-5-5', measure, breakdown })
}

describe('alerts', () => {
  test('asks for the threshold by how full the capped window is, not the model window', () => {
    expect(wantsThreshold(cappedAt(250_000, 'claude-sonnet-5-5', null), KNOWN)).toBe(true)
    expect(wantsThreshold(cappedAt(200_000, 'claude-sonnet-5-5', null), KNOWN)).toBe(false)
  })

  test('trusts a threshold only from a breakdown counted on the current model', () => {
    expect(thresholdOf(cappedAt(250_000, 'claude-sonnet-5-5', 367_000), KNOWN)).toBe(367_000)
    expect(thresholdOf(cappedAt(250_000, 'claude-opus-5-5', 367_000), { ...KNOWN, threshold: 167_000 })).toBe(167_000)
  })

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

  test('warns once a period when the spend pace goes past the limit and once at 90%', () => {
    const base = { nowMs: NOW_MS, local: local(), daemon: OK, known: EMPTY_ALERTED }
    const over = snapshot({ limits: [{ ...spendWindow(40), pace: { pct_per_hour: 0.2, projected_pct: 130, full_at_ms: NOW_MS + 10 * DAY_MS } }] })
    const first = alertsOf({ ...base, snapshot: over })
    expect(first.alerts.map(a => a.kind)).toEqual(['spend'])
    expect(first.alerts[0]?.text).toBe('At this pace the spend limit runs out in 10d, before its reset in 20d 15h')
    expect(alertsOf({ ...base, snapshot: over, known: first.next }).alerts).toEqual([])
    const high = alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(91)] }), known: first.next })
    expect(high.alerts.map(a => a.text)).toEqual(['91% of the spend limit is used, it resets in 20d 15h'])
    expect(alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(95)] }), known: high.next }).alerts).toEqual([])
  })

  test('the spend alerts fire again in the next period and name a budget as such', () => {
    const base = { nowMs: NOW_MS, local: local(), daemon: OK, known: EMPTY_ALERTED }
    const fired = alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(91)] }) }).next
    const nextPeriod = Date.parse('2026-12-01T00:00:00Z')
    expect(alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(92, 'spend_limit', nextPeriod)] }), known: fired }).alerts.map(a => a.kind)).toEqual(['spend'])
    const budget = snapshot({ limits: [{ ...spendWindow(50, 'budget'), pace: { pct_per_hour: 0.2, projected_pct: 130, full_at_ms: NOW_MS + 10 * DAY_MS + 3 * 60 * MINUTE_MS } }] })
    expect(alertsOf({ ...base, snapshot: budget }).alerts.map(a => a.text)).toEqual(['At this pace the budget runs out in 10d 3h, before its reset in 20d 15h'])
    const late = snapshot({ limits: [{ ...spendWindow(50), pace: { pct_per_hour: 0.2, projected_pct: 130, full_at_ms: PERIOD_END_MS + DAY_MS } }] })
    expect(alertsOf({ ...base, snapshot: late }).alerts).toEqual([])
    expect(alertsOf({ ...base, snapshot: snapshot({ limits: [spendWindow(89)] }) }).alerts).toEqual([])
  })

  test('a used up spend limit gets the high toast and no pace toast', () => {
    const base = { nowMs: NOW_MS, local: local(), daemon: OK, known: EMPTY_ALERTED }
    const usedUp = snapshot({ limits: [{ ...spendWindow(104), pace: { pct_per_hour: 0.4, projected_pct: 150, full_at_ms: null } }] })
    expect(alertsOf({ ...base, snapshot: usedUp }).alerts.map(a => a.text)).toEqual(['104% of the spend limit is used, it resets in 20d 15h'])
    const olderDaemon = snapshot({ limits: [{ ...spendWindow(104), pace: { pct_per_hour: 0.4, projected_pct: 150, full_at_ms: NOW_MS } }] })
    expect(alertsOf({ ...base, snapshot: olderDaemon }).alerts.map(a => a.kind)).toEqual(['spend'])
    expect(alertsOf({ ...base, snapshot: olderDaemon }).alerts[0]?.text.startsWith('104%')).toBe(true)
  })

  test('a full 5h window gets no pace toast', () => {
    const full = pacing(0, 2 * 60 * MINUTE_MS)
    const five = { ...(full.live.limits[0] as LimitWindow), used_pct: 100 }
    const alone = local({ measure: { ...local().measure!, rateLimits: [] } })
    expect(alertsOf(input({ local: alone, snapshot: snapshot({ limits: [five] }) })).alerts).toEqual([])
  })

  test('one account spending its limit does not mute the toast of another', () => {
    const base = { nowMs: NOW_MS, local: local(), daemon: OK, known: EMPTY_ALERTED }
    const of = (id: string) => snapshot({ account: { id, label: id, kind: 'oauth' }, limits: [spendWindow(91)] })
    const work = alertsOf({ ...base, snapshot: of('oauth:work') })
    expect(work.alerts.map(a => a.kind)).toEqual(['spend'])
    const me = alertsOf({ ...base, snapshot: of('oauth:me'), known: work.next })
    expect(me.alerts.map(a => a.kind)).toEqual(['spend'])
    expect(alertsOf({ ...base, snapshot: of('oauth:work'), known: me.next }).alerts).toEqual([])
    expect(alertsOf({ ...base, snapshot: of('oauth:me'), known: me.next }).alerts).toEqual([])
  })

  test('a new session keeps the spend alerts of the period and forgets the rest', () => {
    const before: Alerted = { ...EMPTY_ALERTED, session: 's0', cacheFor: 1, paceFor: 2, spendFor: { '': PERIOD_END_MS }, spendHighFor: { '': PERIOD_END_MS } }
    expect(ownAlerts(before, 's1')).toEqual({ ...EMPTY_ALERTED, session: 's1', spendFor: { '': PERIOD_END_MS }, spendHighFor: { '': PERIOD_END_MS } })
    const known = { ...before, session: 's0' }
    expect(alertsOf(input({ snapshot: snapshot({ limits: [spendWindow(91)] }), known })).alerts).toEqual([])
  })

  test('a new session starts with nothing alerted', () => {
    const before: Alerted = { session: 's0', model: null, cacheFor: 1, paceFor: 2, spendFor: {}, spendHighFor: {}, contextFired: true, threshold: 3, thresholdAsked: true }
    expect(ownAlerts(before, 's1')).toEqual({ ...EMPTY_ALERTED, session: 's1' })
    expect(ownAlerts(before, 's0')).toBe(before)
    expect(alertsOf(input({ snapshot: expiring(59_000), known: { ...before, cacheFor: NOW_MS + 59_000 } })).alerts.map(alert => alert.kind)).toEqual(['cache'])
  })

  test('another model forgets the threshold learned for the previous one and warns about its own context again', () => {
    const learned: Alerted = { ...KNOWN, model: 'claude-opus-5-5', threshold: 167_000, thresholdAsked: true, contextFired: true }
    const switched = local({ ...near(170_000, null), model: 'claude-sonnet-5-5[1m]' })
    expect(ownAlerts(learned, 's1', 'claude-sonnet-5-5[1m]')).toEqual({ ...learned, model: 'claude-sonnet-5-5[1m]', threshold: null, thresholdAsked: false, contextFired: false })
    const base = near(930_000, 1_000_000)
    const larger = local({ ...base, model: 'claude-sonnet-5-5[1m]', breakdown: { ...base.breakdown!, model: 'claude-sonnet-5-5[1m]', max_tokens: 1_000_000 } })
    expect(alertsOf(input({ local: larger, known: learned })).alerts.map(alert => alert.kind)).toEqual(['context'])
    expect(ownAlerts(learned, 's1', 'claude-opus-5-5')).toBe(learned)
    expect(ownAlerts({ ...KNOWN, threshold: 167_000 }, 's1', 'claude-opus-5-5')).toEqual({ ...KNOWN, threshold: 167_000, model: 'claude-opus-5-5' })
    expect(alertsOf(input({ local: switched, known: learned })).alerts).toEqual([])
    expect(wantsThreshold(switched, learned)).toBe(true)
  })

  test('decided alerts are merged onto what changed meanwhile instead of replacing it', () => {
    const before: Alerted = { ...KNOWN, model: 'claude-opus-5-5' }
    const decided: Alerted = { ...before, cacheFor: NOW_MS + 59_000 }
    const meanwhile: Alerted = { ...before, thresholdAsked: true, threshold: 167_000 }
    expect(merged(meanwhile, before, decided)).toEqual({ ...meanwhile, cacheFor: NOW_MS + 59_000 })
    const newSession: Alerted = { ...EMPTY_ALERTED, session: 's2' }
    expect(merged(meanwhile, before, newSession)).toEqual(newSession)
  })
})
