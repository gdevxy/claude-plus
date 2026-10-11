import type { Alerted, Daemon, Local, Snapshot } from '../types'
import { duration, money, tokens } from './format'
import { LAST_MINUTE_MS, RESET_TOLERANCE_MS, cacheClock, current, limitsOf } from './state'

export const CONTEXT_ALERT = 0.9
export const CONTEXT_REARM = 0.8
export const CONTEXT_ASK_PCT = 60
export const ALERT_TOAST_MS = 8000
export const CHIME = 'sounds/chime.wav'

export const EMPTY_ALERTED: Alerted = { session: null, model: null, cacheFor: null, paceFor: null, contextFired: false, threshold: null, thresholdAsked: false }

export type Alert = { kind: 'cache' | 'pace' | 'context'; text: string }

export type AlertInput = { nowMs: number; snapshot: Snapshot | null; local: Local; daemon: Daemon; known: Alerted }

export function ownAlerts(known: Alerted, session: string | null, model: string | null = known.model): Alerted {
  if (known.session !== session) return { ...EMPTY_ALERTED, session, model }
  if (model === null || known.model === model) return known
  if (known.model === null) return { ...known, model }
  return { ...known, model, threshold: null, thresholdAsked: false }
}

export function merged(current: Alerted, before: Alerted, after: Alerted): Alerted {
  if (after.session !== before.session) return after
  const changed = (Object.keys(after) as (keyof Alerted)[]).filter(key => after[key] !== before[key])
  return changed.reduce((result, key) => ({ ...result, [key]: after[key] }), current)
}

export function thresholdOf(local: Local, known: Alerted): number | null {
  return local.breakdown?.auto_compact_threshold ?? known.threshold
}

export function wantsThreshold(local: Local, known: Alerted): boolean {
  const percent = local.measure?.contextPercent ?? null
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
    const price = cold === null ? '' : ` rewarming later costs ~${money(cold)}`
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
