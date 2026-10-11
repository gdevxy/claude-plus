import type { Alerted, Daemon, Local, Snapshot } from '../types'
import { approx, duration, percent, tokens } from './format'
import { LAST_MINUTE_MS, RESET_TOLERANCE_MS, cacheClock, contextGauge, current, limitsOf, ownBreakdown } from './state'

export const CONTEXT_ALERT = 0.9
export const CONTEXT_REARM = 0.8
export const CONTEXT_ASK_PCT = 60
export const ALERT_TOAST_MS = 8000
export const CHIME = 'sounds/chime.wav'

export const EMPTY_ALERTED: Alerted = { session: null, model: null, cacheFor: null, paceFor: null, spendFor: null, spendHighFor: null, contextFired: false, threshold: null, thresholdAsked: false }

export type Alert = { kind: 'cache' | 'pace' | 'spend' | 'context'; text: string }

export const SPEND_HIGH_PCT = 90

export type AlertInput = { nowMs: number; snapshot: Snapshot | null; local: Local; daemon: Daemon; known: Alerted }

export function ownAlerts(known: Alerted, session: string | null, model: string | null = known.model): Alerted {
  if (known.session !== session) return { ...EMPTY_ALERTED, session, model, spendFor: known.spendFor, spendHighFor: known.spendHighFor }
  if (model === null || known.model === model) return known
  if (known.model === null) return { ...known, model }
  return { ...known, model, threshold: null, thresholdAsked: false, contextFired: false }
}

export function merged(current: Alerted, before: Alerted, after: Alerted): Alerted {
  if (after.session !== before.session) return after
  const changed = (Object.keys(after) as (keyof Alerted)[]).filter(key => after[key] !== before[key])
  return changed.reduce((result, key) => ({ ...result, [key]: after[key] }), current)
}

export function thresholdOf(local: Local, known: Alerted): number | null {
  return ownBreakdown(local)?.auto_compact_threshold ?? known.threshold
}

export function wantsThreshold(local: Local, known: Alerted): boolean {
  const percent = contextGauge(local)?.percent ?? null
  const own = ownAlerts(known, local.session, local.model)
  return percent !== null && percent >= CONTEXT_ASK_PCT && thresholdOf(local, own) === null && !own.thresholdAsked
}

export function alertsOf(input: AlertInput): { alerts: Alert[]; next: Alerted } {
  const known = ownAlerts(input.known, input.local.session, input.local.model)
  const alerts: Alert[] = []
  const next: Alerted = { ...known }
  const snapshot = current(input)
  const clock = cacheClock(snapshot, input.local, input.nowMs)
  if (clock.state === 'warm' && clock.remainingMs <= LAST_MINUTE_MS && !input.local.working && known.cacheFor !== clock.expiresAtMs) {
    const cold = snapshot?.live.forecast?.cold_usd ?? null
    const price = cold === null ? '' : ` rewarming later costs ${approx(cold)}`
    alerts.push({ kind: 'cache', text: `Cache goes cold in under a minute ✻${price}` })
    next.cacheFor = clock.expiresAtMs
  }
  const five = limitsOf({ ...input, snapshot }).find(limit => limit.window === 'five_hour')
  const full = five?.pace?.full_at_ms ?? null
  const resets = five?.resets_at_ms ?? null
  if (full !== null && resets !== null && full < resets && (known.paceFor === null || Math.abs(known.paceFor - resets) > RESET_TOLERANCE_MS)) {
    const left = duration(Math.max(0, full - input.nowMs))
    alerts.push({ kind: 'pace', text: `At this pace the 5h limit runs out in ${left}, before its reset in ${duration(resets - input.nowMs)}` })
    next.paceFor = resets
  }
  const spend = limitsOf({ ...input, snapshot }).find(limit => limit.window === 'spend_limit' || limit.window === 'budget')
  const spendResets = spend?.resets_at_ms ?? null
  const spendFull = spend?.pace?.full_at_ms ?? null
  const unseen = (seen: number | null) => spendResets !== null && (seen === null || Math.abs(seen - spendResets) > RESET_TOLERANCE_MS)
  const name = spend?.window === 'budget' ? 'budget' : 'spend limit'
  if (spendFull !== null && spendResets !== null && spendFull < spendResets && unseen(known.spendFor)) {
    alerts.push({ kind: 'spend', text: `At this pace the ${name} runs out in ${duration(Math.max(0, spendFull - input.nowMs))}, before its reset in ${duration(spendResets - input.nowMs)}` })
    next.spendFor = spendResets
  }
  if (spend !== undefined && spendResets !== null && spend.used_pct >= SPEND_HIGH_PCT && unseen(known.spendHighFor)) {
    alerts.push({ kind: 'spend', text: `${percent(spend.used_pct)} of the ${name} is used, it resets in ${duration(spendResets - input.nowMs)}` })
    next.spendHighFor = spendResets
  }
  const used = input.local.measure?.contextTokens ?? null
  const threshold = thresholdOf(input.local, known)
  if (used !== null && threshold !== null && threshold > 0) {
    const ratio = used / threshold
    if (ratio >= CONTEXT_ALERT && !known.contextFired) {
      alerts.push({ kind: 'context', text: `Context is close to auto-compact, ${tokens(used)} of ${tokens(threshold)} ✻ /compact now to choose what stays` })
      next.contextFired = true
    }
    if (ratio < CONTEXT_REARM) next.contextFired = false
  }
  return { alerts, next }
}
