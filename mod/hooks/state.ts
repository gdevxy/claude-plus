import type { ThemeKey, TurnUsage } from 'claude-code'

import type { Daemon, DaemonStatus, LimitWindow, Local, Motion, Snapshot, Step } from '../types'
import {
  bar,
  countdown,
  duration,
  finePercent,
  modelName,
  money,
  percent,
  spread,
  tokens,
  windowName,
} from './format'

export const PADDING = 1
export const MIN_CONTENT = 24
export const EASE_MS = 300
export const FRAME_MS = 50
export const BREATH_MS = 1000
export const QUICK_BREATH_MS = 500
export const TICK_MS = 1000
export const LAST_MINUTE_MS = 60_000
export const HOUR_MS = 3_600_000
export const FIVE_MINUTES_MS = 300_000
export const CONTEXT_WARNING = 80
export const CONTEXT_ERROR = 95
export const LIMIT_WARNING = 80
export const WHY_COLUMNS = 2
export const DETAIL_INDENT = 8

export type Segment = { text: string; color?: ThemeKey; dim?: true; bold?: true }
export type Line = { key: string; segments: Segment[]; why?: string; wrap?: true; indent?: number }

export type ScreenInput = {
  width: number
  nowMs: number
  animations: boolean
  snapshot: Snapshot | null
  local: Local
  daemon: Daemon
  expanded: readonly string[]
  motions: Readonly<Record<string, Motion>>
}

export type CacheClock = {
  state: 'warm' | 'cold' | 'none'
  ttlMs: number
  assumed: boolean
  expiresAtMs: number | null
  remainingMs: number
}

export const EMPTY_LOCAL: Local = {
  session: null,
  model: null,
  turns: 0,
  working: false,
  measure: null,
  step: null,
}

export const MISSING_EXIT = 127

export function stepOf(usage: TurnUsage, atMs: number): Step {
  return {
    atMs,
    model: usage.model,
    input: usage.input_tokens,
    output: usage.output_tokens,
    cacheRead: usage.cache_read_input_tokens,
    cacheWrite: usage.cache_creation_input_tokens,
  }
}

export function ensureOutcome(exitCode: number, stderr: string): Daemon {
  if (exitCode === 0) return { status: 'ok', detail: null }
  if (exitCode === MISSING_EXIT) return { status: 'missing', detail: null }
  return { status: 'failed', detail: firstLine(stderr) ?? `exit code ${exitCode}` }
}

export function ensureError(error: unknown): Daemon {
  const message = error instanceof Error ? error.message : String(error)
  const status: DaemonStatus = /ENOENT|not found/i.test(message) ? 'missing' : 'failed'
  return { status, detail: status === 'failed' ? firstLine(message) ?? message : null }
}

export function toggled(list: readonly string[], key: string): string[] {
  return list.includes(key) ? list.filter(item => item !== key) : [...list, key]
}

function firstLine(text: string): string | null {
  const line = text.split('\n').map(part => part.trim()).find(part => part.length > 0)
  return line === undefined ? null : line.replace(/^error:\s*/i, '')
}

export function current(input: Pick<ScreenInput, 'snapshot' | 'local'>): Snapshot | null {
  const { snapshot, local } = input
  return snapshot !== null && (local.session === null || snapshot.session === local.session) ? snapshot : null
}

export function cacheClock(snapshot: Snapshot | null, local: Local, nowMs: number): CacheClock {
  const cache = snapshot?.live.cache
  const subscribed = (local.measure?.rateLimits.length ?? 0) > 0
  const ttlMs =
    cache !== undefined ? (cache.ttl === '1h' ? HOUR_MS : FIVE_MINUTES_MS) : subscribed ? HOUR_MS : FIVE_MINUTES_MS
  const step = local.step
  const touched = step !== null && (step.cacheRead > 0 || step.cacheWrite > 0) ? step.atMs + ttlMs : null
  const candidates = [cache?.expires_at_ms ?? null, touched].filter((at): at is number => at !== null)
  const expiresAtMs = candidates.length > 0 ? Math.max(...candidates) : null
  const remainingMs = expiresAtMs === null ? 0 : Math.max(0, expiresAtMs - nowMs)
  return {
    state: expiresAtMs === null ? 'none' : remainingMs > 0 ? 'warm' : 'cold',
    ttlMs,
    assumed: cache === undefined || cache.ttl_source === 'assumed',
    expiresAtMs,
    remainingMs,
  }
}

export function heartbeat(clock: CacheClock, nowMs: number, animations: boolean, working: boolean): Segment {
  if (clock.state !== 'warm') return { text: '·', dim: true }
  const urgent = clock.remainingMs <= LAST_MINUTE_MS
  const color: ThemeKey = urgent ? 'warning' : 'claude'
  if (!animations || working) return { text: '✻', color }
  if (urgent) {
    return Math.floor(nowMs / QUICK_BREATH_MS) % 2 === 0
      ? { text: '✻', color, bold: true }
      : { text: '✻', color, dim: true }
  }
  const phase = Math.floor(nowMs / BREATH_MS) % 4
  if (phase === 0) return { text: '✻', color, dim: true }
  if (phase === 2) return { text: '✻', color, bold: true }
  return { text: '✻', color }
}

export function eased(motion: Motion, nowMs: number): number {
  const progress = (nowMs - motion.startMs) / EASE_MS
  if (progress >= 1) return motion.to
  if (progress <= 0) return motion.from
  return motion.from + (motion.to - motion.from) * (1 - (1 - progress) ** 3)
}

export function shown(motions: Readonly<Record<string, Motion>>, key: string, target: number, nowMs: number): number {
  const motion = motions[key]
  return motion !== undefined && motion.to === target ? eased(motion, nowMs) : target
}

export function isMoving(motions: Readonly<Record<string, Motion>>, nowMs: number): boolean {
  return Object.values(motions).some(motion => nowMs - motion.startMs < EASE_MS)
}

export function settled(motions: Readonly<Record<string, Motion>>, nowMs: number): Record<string, Motion> {
  return Object.fromEntries(Object.entries(motions).filter(([, motion]) => nowMs - motion.startMs < EASE_MS))
}

export function easables(snapshot: Snapshot | null, local: Local): Record<string, number> {
  const values: Record<string, number> = {}
  const cost = snapshot?.totals.cost_usd ?? local.measure?.costUsd ?? null
  if (cost !== null) values.cost = cost
  const context = local.measure?.contextPercent ?? null
  if (context !== null) values.context = context
  const hit = snapshot?.live.cache.hit_ratio ?? null
  if (hit !== null) values.hit = hit * 100
  for (const limit of limitsOf(snapshot, local)) values[`limit:${limit.window}`] = limit.used_pct
  return values
}

export function retarget(
  motions: Readonly<Record<string, Motion>>,
  before: Record<string, number>,
  after: Record<string, number>,
  nowMs: number,
  animations: boolean,
): Record<string, Motion> {
  if (!animations) return {}
  const next: Record<string, Motion> = {}
  for (const [key, to] of Object.entries(after)) {
    const from = before[key]
    const running = motions[key]
    if (from === undefined || from === to) {
      if (running !== undefined && running.to === to && nowMs - running.startMs < EASE_MS) next[key] = running
      continue
    }
    next[key] = { from: shown(motions, key, from, nowMs), to, startMs: nowMs }
  }
  return next
}

export function tickPeriod(input: Pick<ScreenInput, 'animations' | 'motions' | 'nowMs' | 'snapshot' | 'local'>): number {
  if (!input.animations) return TICK_MS
  if (isMoving(input.motions, input.nowMs)) return FRAME_MS
  const clock = cacheClock(current(input), input.local, input.nowMs)
  return clock.state === 'warm' && clock.remainingMs <= LAST_MINUTE_MS ? QUICK_BREATH_MS : TICK_MS
}

export function statusText(input: Pick<ScreenInput, 'snapshot' | 'local' | 'nowMs'>): string | undefined {
  const snapshot = current(input)
  const parts: string[] = []
  const cost = snapshot?.totals.cost_usd ?? input.local.measure?.costUsd ?? null
  if (cost !== null) parts.push(money(cost))
  const clock = cacheClock(snapshot, input.local, input.nowMs)
  if (clock.state === 'warm') parts.push(`cache ${countdown(clock.remainingMs)}`)
  if (clock.state === 'cold') parts.push('cache cold')
  const five = limitsOf(snapshot, input.local).find(limit => limit.window === 'five_hour')
  if (five !== undefined) parts.push(`5h ${percent(five.used_pct)}`)
  return parts.length > 0 ? parts.join(' · ') : undefined
}

export function limitsOf(snapshot: Snapshot | null, local: Local): LimitWindow[] {
  if (snapshot !== null && snapshot.live.limits.length > 0) return snapshot.live.limits
  return (local.measure?.rateLimits ?? []).map(limit => ({
    window: limit.kind,
    used_pct: limit.percentUsed,
    resets_at_ms: limit.resetsAtMs,
    sampled_at_ms: local.measure?.atMs ?? 0,
    reset_since_sample: false,
    pace: null,
    calibration: { family: 'other', pairs: 0, needed: 20, pct_per_usd: null },
  }))
}

export function screen(input: ScreenInput): Line[] {
  const width = Math.max(MIN_CONTENT, input.width - 2 * PADDING)
  const snapshot = current(input)
  const sections = [
    notice(input),
    headline(input, snapshot, width),
    context(input, width),
    cache(input, snapshot, width),
    forecast(input, snapshot, width),
    session(input, snapshot, width),
    limits(input, snapshot, width),
  ].filter(section => section.length > 0)
  return sections.flatMap((section, index) => (index === 0 ? section : [gap(index), ...section]))
}

function gap(index: number): Line {
  return { key: `gap-${index}`, segments: [{ text: ' ' }] }
}

function plain(key: string, text: string, style: Style = {}): Line {
  return { key, segments: [{ text, ...style }] }
}

function prose(key: string, text: string, style: Style = {}, indent = 0): Line {
  const line: Line = { key, segments: [{ text, ...style }], wrap: true }
  if (indent > 0) line.indent = indent
  return line
}

type Style = Omit<Segment, 'text'>

function pair(key: string, left: string, right: string, width: number, rightStyle: Style = {}, leftStyle: Style = {}): Line {
  const text = spread(left, right, width)
  return {
    key,
    segments: [
      { text: text.slice(0, text.length - right.length), ...leftStyle },
      { text: right, ...rightStyle },
    ],
  }
}

function heading(key: string, title: string, width: number, why?: string): Line {
  const line: Line = { key, segments: [{ text: title.padEnd(why === undefined ? width : width - WHY_COLUMNS), bold: true }] }
  if (why !== undefined) line.why = why
  return line
}

function meter(key: string, label: string, fraction: number, value: string, width: number, color: ThemeKey): Line {
  const labelled = label.length > 0 ? `${label} ` : ''
  const cells = bar(fraction, width - labelled.length - value.length - 1)
  return {
    key,
    segments: [
      { text: labelled },
      { text: cells.filled, color },
      { text: cells.track, dim: true },
      { text: ` ${value}` },
    ],
  }
}

function notice(input: ScreenInput): Line[] {
  const { status, detail } = input.daemon
  if (status === 'ok') return []
  if (status === 'starting') return [prose('daemon', 'Starting the claude-plus daemon…', { dim: true })]
  if (status === 'rebuilding') return [prose('daemon', 'Rebuilding the claude-plus index… numbers return in a moment', { dim: true })]
  if (status === 'missing') {
    return [prose('daemon', 'claude-plus is not installed yet. Install it, or set its path in the plugin options', { color: 'warning' })]
  }
  if (status === 'unreachable') {
    return [prose('daemon', 'The daemon is not answering · run claude-plus daemon --ensure', { color: 'warning' })]
  }
  return [prose('daemon', `The daemon could not start: ${detail ?? 'no reason given'}`, { color: 'error' })]
}

function headline(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const id = snapshot?.live.model ?? input.local.model
  const effort = snapshot?.live.effort ?? null
  const name = id === null ? 'Claude' : modelName(id)
  const left = effort === null ? name : `${name} · ${effort}`
  const cost = snapshot?.totals.cost_usd ?? input.local.measure?.costUsd ?? null
  const lines: Line[] = []
  if (cost !== null) {
    const value = money(shown(input.motions, 'cost', cost, input.nowMs))
    lines.push(pair('headline', left, value, width, { color: 'claude', bold: true }))
  } else {
    lines.push(plain('headline', left))
  }
  const unpriced = snapshot?.totals.unpriced_requests ?? 0
  if (unpriced > 0) {
    const noun = unpriced === 1 ? 'request on an unknown model is' : 'requests on unknown models are'
    lines.push(prose('unpriced', `${unpriced} ${noun} not priced`, { dim: true }))
  }
  return lines
}

function context(input: ScreenInput, width: number): Line[] {
  const measure = input.local.measure
  if (measure === null || measure.contextPercent === null) {
    return [plain('context', 'Context', { bold: true }), prose('context-empty', 'Context appears after your first prompt', { dim: true })]
  }
  const used = shown(input.motions, 'context', measure.contextPercent, input.nowMs)
  const amount =
    measure.contextTokens === null ? tokens(measure.contextWindow) : `${tokens(measure.contextTokens)} / ${tokens(measure.contextWindow)}`
  const color: ThemeKey = used >= CONTEXT_ERROR ? 'error' : used >= CONTEXT_WARNING ? 'warning' : 'text'
  return [
    pair('context', 'Context', amount, width, {}, { bold: true }),
    meter('context-bar', '', used / 100, percent(used).padStart(4), width, color),
  ]
}

function cache(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const clock = cacheClock(snapshot, input.local, input.nowMs)
  const glyph = heartbeat(clock, input.nowMs, input.animations, input.local.working)
  const ttl = `${clock.assumed ? '~' : ''}${clock.ttlMs === HOUR_MS ? '1h' : '5m'}`
  const forecast = snapshot?.live.forecast ?? null
  const lines: Line[] = []
  if (clock.state === 'none') {
    lines.push(heading('cache', 'Cache', width))
    lines.push(prose('cache-note', 'Cache starts with your first prompt ✻', { dim: true }))
    return lines
  }
  const state = clock.state === 'warm' ? 'warm' : 'cold'
  const right = clock.state === 'warm' ? `${countdown(clock.remainingMs)} left` : 'expired'
  const title = 'Cache '
  const left = `${title}${glyph.text} ${state} · ${ttl}`
  const padded = spread(left, right, width - WHY_COLUMNS)
  const urgent = clock.state === 'warm' && clock.remainingMs <= LAST_MINUTE_MS
  lines.push({
    key: 'cache',
    why: 'cache',
    segments: [
      { text: title, bold: true },
      glyph,
      { text: padded.slice(title.length + glyph.text.length, padded.length - right.length) },
      { text: right, ...(clock.state === 'warm' ? { color: urgent ? 'warning' : 'success' } : { dim: true }) },
    ],
  })
  const cells = bar(clock.remainingMs / clock.ttlMs, width)
  lines.push({
    key: 'cache-bar',
    segments: [
      { text: cells.filled, color: urgent ? 'warning' : 'success' },
      { text: cells.track, dim: true },
    ],
  })
  const live = snapshot?.live.cache
  if (clock.state === 'cold' && forecast?.cold_usd != null) {
    lines.push(prose('cache-note', `Cache went cold while you were away, rewarming will cost ~${money(forecast.cold_usd)}`))
  } else if (live !== undefined && live.saved_usd >= 0.01) {
    lines.push(prose('cache-note', `Cache hits saved you ${money(live.saved_usd)} this session`, { color: 'success' }))
  }
  if (live !== undefined && live.hit_ratio !== null) {
    const hit = shown(input.motions, 'hit', live.hit_ratio * 100, input.nowMs)
    lines.push(pair('cache-hits', '  Hit ratio', percent(hit), width, {}, { dim: true }))
  }
  if (live !== undefined && live.rebuilds > 0) {
    const value = `${live.rebuilds} · ~${money(live.rebuild_usd)}`
    lines.push({ ...pair('cache-rebuilds', '  Rebuilds', value, width - WHY_COLUMNS, {}, { dim: true }), why: 'rebuilds' })
  }
  if (input.expanded.includes('cache')) lines.push(...cacheWhy(clock))
  if (input.expanded.includes('rebuilds') && live?.last_rebuild != null) {
    lines.push(prose('explain-rebuilds', rebuildWhy(live.last_rebuild.cause, input.nowMs - live.last_rebuild.ts_ms), { dim: true }))
  }
  return lines
}

function cacheWhy(clock: CacheClock): Line[] {
  const ttl = clock.ttlMs === HOUR_MS ? '1 hour' : '5 minutes'
  const source = clock.assumed
    ? `No cache write has been read from the transcript yet, so the ${ttl} lifetime is assumed from your plan.`
    : `The last cache write in the transcript asked for a ${ttl} lifetime.`
  return [prose('explain-cache', `${source} Every request that reads or writes the cache starts the countdown again.`, { dim: true })]
}

function rebuildWhy(cause: string, agoMs: number): string {
  const reasons: Record<string, string> = {
    idle: 'the cache expired during an idle gap',
    model_switch: 'the model changed, and each model keeps its own cache',
    prefix_changed: 'the injected context changed (tools, skills or memory)',
    unknown: 'no cause could be told from the transcript',
  }
  return `The last rebuild, ${duration(agoMs)} ago, wrote the prompt again because ${reasons[cause] ?? reasons.unknown}.`
}

function forecast(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const next = snapshot?.live.forecast ?? null
  if (next === null || (next.warm_usd === null && next.cold_usd === null)) return []
  const clock = cacheClock(snapshot, input.local, input.nowMs)
  const cost = (usd: number | null, pct: number | null) =>
    usd === null ? 'price unknown' : pct === null ? `~${money(usd)}` : `~${money(usd)} · ${finePercent(pct)} of 5h`
  const warmFirst = clock.state === 'warm'
  const lines: Line[] = [
    heading('next', 'Next turn', width, 'forecast'),
    pair('next-warm', '  if warm', cost(next.warm_usd, next.warm_pct_5h), width, warmFirst ? {} : { dim: true }, { dim: true }),
    pair('next-cold', '  if cold', cost(next.cold_usd, next.cold_pct_5h), width, warmFirst ? { dim: true } : {}, { dim: true }),
  ]
  if (next.alternative !== null) {
    const value = `~${money(next.alternative.first_usd)}, then ~${money(next.alternative.next_usd)}`
    lines.push(pair('next-alternative', `  on ${modelName(next.alternative.model)}`, value, width, { dim: true }, { dim: true }))
  }
  if (input.expanded.includes('forecast')) {
    lines.push(
      prose(
        'explain-forecast',
        `Your ${tokens(next.prefix_tokens)} token prompt is read from the cache when warm and written again when cold. ` +
          `${tokens(next.new_tokens)} new and ${tokens(next.output_tokens)} output tokens are the medians of your last ${next.samples} requests.`,
        { dim: true },
      ),
    )
  }
  return lines
}

function session(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const lines: Line[] = [heading('session', 'Session', width)]
  if (snapshot !== null) {
    const t = snapshot.totals.tokens
    const prompt = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h
    lines.push(pair('session-tokens', '  Tokens in / out', `${tokens(prompt)} / ${tokens(t.output)}`, width, {}, { dim: true }))
    lines.push(pair('session-turns', '  Turns · subagents', `${input.local.turns} · ${snapshot.totals.subagents}`, width, {}, { dim: true }))
    if (snapshot.totals.reconciled_usd >= 0.01) {
      lines.push(pair('session-reconciled', '  Reconciled', money(snapshot.totals.reconciled_usd), width, { dim: true }, { dim: true }))
    }
    return lines
  }
  const step = input.local.step
  lines.push(pair('session-turns', '  Turns', String(input.local.turns), width, {}, { dim: true }))
  if (step !== null) {
    const prompt = step.input + step.cacheRead + step.cacheWrite
    lines.push(pair('session-step', '  Last request', `${tokens(prompt)} in · ${tokens(step.output)} out`, width, {}, { dim: true }))
  }
  return lines
}

function limits(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const windows = limitsOf(snapshot, input.local)
  if (windows.length === 0) return []
  const lines: Line[] = [heading('limits', 'Plan limits', width, 'limits')]
  for (const limit of windows) {
    const used = shown(input.motions, `limit:${limit.window}`, limit.used_pct, input.nowMs)
    const color: ThemeKey = used >= LIMIT_WARNING ? 'warning' : 'text'
    lines.push(meter(`limit-${limit.window}`, `  ${windowName(limit.window).padEnd(5)}`, used / 100, percent(used).padStart(4), width, color))
    const detail = limitDetail(limit, input.nowMs)
    if (detail !== null) lines.push(detail)
  }
  const five = windows.find(limit => limit.window === 'five_hour')
  if (snapshot !== null && five !== undefined && five.calibration.pct_per_usd === null) {
    lines.push(prose('limits-learning', 'Learning how your plan counts tokens… a few more turns ✻', { dim: true }))
  }
  if (input.expanded.includes('limits')) {
    const learned = five?.calibration
    const how =
      learned?.pct_per_usd != null
        ? `From ${learned.pairs} readings, 1% of your 5h window is about ${money(1 / learned.pct_per_usd)} of API-priced usage on ${familyName(learned.family)}.`
        : `Each turn pairs the percent Claude Code reports with what it would cost on the API; ${learned?.pairs ?? 0} of ${learned?.needed ?? 20} readings so far.`
    lines.push(prose('explain-limits', `${how} The pace projects the last 30 minutes to the reset.`, { dim: true }))
  }
  return lines
}

function familyName(family: string): string {
  return family.charAt(0).toUpperCase() + family.slice(1)
}

function limitDetail(limit: LimitWindow, nowMs: number): Line | null {
  const key = `limit-${limit.window}-detail`
  const parts: string[] = []
  if (limit.resets_at_ms !== null) parts.push(`resets in ${duration(Math.max(0, limit.resets_at_ms - nowMs))}`)
  const pace = limit.pace
  if (pace?.full_at_ms != null) {
    parts.push(`full in ${duration(Math.max(0, pace.full_at_ms - nowMs))} at this pace`)
    return prose(key, parts.join(' · '), { color: 'warning' }, DETAIL_INDENT)
  }
  if (pace?.projected_pct != null) parts.push(`on pace for ${percent(pace.projected_pct)}`)
  return parts.length > 0 ? prose(key, parts.join(' · '), { dim: true }, DETAIL_INDENT) : null
}
