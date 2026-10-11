import type { ThemeKey, TurnUsage } from 'claude-code'

import type { Daemon, DaemonStatus, LimitWindow, LiveContext, Local, ModelCost, Motion, Snapshot, Step } from '../types'
import type { Grid } from './chart'
import {
  approx,
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
export const BREATH_MS = 4000
export const QUICK_BREATH_MS = 1000
export const BREATH_FRAME_MS = 100
export const HEARTBEAT = '✻'
export const MARK = '│'
export const TICK_MS = 1000
export const LAST_MINUTE_MS = 60_000
export const HOUR_MS = 3_600_000
export const FIVE_MINUTES_MS = 300_000
export const CONTEXT_WARNING = 80
export const LIMIT_WARNING = 80
export const WHY_COLUMNS = 2
export const DETAIL_INDENT = 8
export const WINDOW_ORDER = ['five_hour', 'seven_day', 'spend_limit']
export const RESET_TOLERANCE_MS = 60_000
export const PAIRS_NEEDED = 20

export type Segment = { text: string; color?: ThemeKey; dim?: true; bold?: true; glow?: number }
export type Line = { key: string; segments: Segment[]; why?: string; toggle?: string; wrap?: true; indent?: number; chart?: Grid }

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

export type ContextGauge = { tokens: number | null; window: number; percent: number | null; modelWindow: number | null; threshold: number | null }

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
  effort: null,
  turns: 0,
  working: false,
  measure: null,
  step: null,
  breakdown: null,
}

export const MISSING_EXIT = 127
export const LEFT_STOPPED_EXIT = 3

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
  if (exitCode === LEFT_STOPPED_EXIT) return { status: 'stopped', detail: null }
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

export type Breath = { color: ThemeKey; anchorMs: number; periodMs: number }

function glyphColor(clock: CacheClock): ThemeKey {
  return clock.remainingMs <= LAST_MINUTE_MS ? 'warning' : 'claude'
}

export function breathOf(clock: CacheClock, animations: boolean, working: boolean): Breath | null {
  if (clock.state !== 'warm' || !animations || working) return null
  const color = glyphColor(clock)
  return { color, anchorMs: clock.expiresAtMs ?? 0, periodMs: color === 'warning' ? QUICK_BREATH_MS : BREATH_MS }
}

export function glowOf(breathing: Breath, nowMs: number): number {
  return breath(nowMs - breathing.anchorMs, breathing.periodMs)
}

export function heartbeat(clock: CacheClock, nowMs: number, animations: boolean, working: boolean): Segment {
  if (clock.state !== 'warm') return { text: '·', dim: true }
  const breathing = breathOf(clock, animations, working)
  if (breathing === null) return { text: HEARTBEAT, color: glyphColor(clock) }
  return { text: HEARTBEAT, color: breathing.color, glow: glowOf(breathing, nowMs) }
}

export function breath(sinceMs: number, periodMs: number): number {
  const phase = (((sinceMs % periodMs) + periodMs) % periodMs) / periodMs
  return Math.round((0.5 - 0.5 * Math.cos(2 * Math.PI * phase)) * 1000) / 1000
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

export type Sources = Pick<ScreenInput, 'snapshot' | 'local' | 'daemon' | 'nowMs'>

function isStale(sources: Sources): boolean {
  return sources.daemon.status !== 'ok'
}

export function costOf(sources: Sources): number | null {
  const kept = sources.snapshot?.totals.cost_usd ?? null
  const measured = sources.local.measure?.costUsd ?? null
  return isStale(sources) ? (measured ?? kept) : (kept ?? measured)
}

function freshest(sources: Sources, kept: string | null, local: string | null): string | null {
  return isStale(sources) ? (local ?? kept) : (kept ?? local)
}

export function easables(sources: Sources): Record<string, number> {
  const values: Record<string, number> = {}
  const cost = costOf(sources)
  if (cost !== null) values.cost = cost
  const context = contextGauge(sources.local)?.percent ?? null
  if (context !== null) values.context = context
  const hit = sources.snapshot?.live.cache.hit_ratio ?? null
  if (hit !== null) values.hit = hit * 100
  for (const limit of limitsOf(sources)) values[`limit:${limit.window}`] = limit.used_pct
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

export type TickInput = Pick<ScreenInput, 'animations' | 'motions' | 'nowMs' | 'snapshot' | 'local'> & { breathing: boolean }

export function tickPeriod(input: TickInput): number {
  if (!input.animations) return TICK_MS
  if (isMoving(input.motions, input.nowMs)) return FRAME_MS
  const clock = cacheClock(current(input), input.local, input.nowMs)
  if (clock.state !== 'warm' || input.local.working || input.breathing) return TICK_MS
  return clock.remainingMs <= LAST_MINUTE_MS ? QUICK_BREATH_MS / 2 : TICK_MS
}

export function nextTickDelay(periodMs: number, anchorMs: number, nowMs: number): number {
  return (((anchorMs - nowMs) % periodMs) + periodMs) % periodMs || periodMs
}

export function statusText(input: Sources): string | undefined {
  const sources = { ...input, snapshot: current(input) }
  const parts: string[] = []
  const cost = costOf(sources)
  if (cost !== null) parts.push(money(cost))
  const clock = cacheClock(sources.snapshot, input.local, input.nowMs)
  if (clock.state === 'warm') parts.push(`cache ${countdown(clock.remainingMs)}`)
  if (clock.state === 'cold') parts.push('cache cold')
  const five = limitsOf(sources).find(limit => limit.window === 'five_hour')
  if (five !== undefined) parts.push(`5h ${percent(five.used_pct)}`)
  return parts.length > 0 ? parts.join(' · ') : undefined
}

export function limitsOf(sources: Sources): LimitWindow[] {
  const kept = sources.snapshot?.live.limits ?? []
  const measure = sources.local.measure
  const measured = (measure?.rateLimits ?? []).map((limit): LimitWindow => {
    const known = kept.find(window => window.window === limit.kind)
    const reset = limit.resetsAtMs !== null && limit.resetsAtMs <= sources.nowMs
    const sameWindow =
      known !== undefined &&
      (known.resets_at_ms === null || limit.resetsAtMs === null
        ? known.resets_at_ms === limit.resetsAtMs
        : Math.abs(known.resets_at_ms - limit.resetsAtMs) <= RESET_TOLERANCE_MS)
    return {
      window: limit.kind,
      used_pct: reset ? 0 : limit.percentUsed,
      resets_at_ms: reset ? null : limit.resetsAtMs,
      sampled_at_ms: measure?.atMs ?? 0,
      reset_since_sample: reset,
      pace: reset || !sameWindow ? null : known.pace,
      calibration: known?.calibration ?? { family: 'other', pairs: 0, needed: PAIRS_NEEDED, pct_per_usd: null },
    }
  })
  const newer = (fresh: LimitWindow, window: LimitWindow) => fresh.window === window.window && fresh.sampled_at_ms > window.sampled_at_ms
  const windows = [
    ...kept.filter(window => !measured.some(fresh => newer(fresh, window))),
    ...measured.filter(fresh => kept.every(window => window.window !== fresh.window || newer(fresh, window))),
  ]
  return windows.sort((a, b) => windowRank(a.window) - windowRank(b.window) || a.window.localeCompare(b.window))
}

function windowRank(kind: string): number {
  const rank = WINDOW_ORDER.indexOf(kind)
  return rank < 0 ? WINDOW_ORDER.length : rank
}

export function screen(input: ScreenInput): Line[] {
  const width = Math.max(MIN_CONTENT, input.width - 2 * PADDING)
  const snapshot = current(input)
  const sections = [
    notice(input, width),
    headline(input, snapshot, width),
    context(input, width),
    cache(input, snapshot, width),
    forecast(input, snapshot, width),
    session(input, snapshot, width),
    limits(input, snapshot, width),
  ].filter(section => section.length > 0)
  return sections.flatMap((section, index) => (index === 0 ? section : [gap(index), ...section]))
}

export function gap(index: number): Line {
  return { key: `gap-${index}`, segments: [{ text: ' ' }] }
}

export function fitting(width: number, ...choices: string[]): string {
  return choices.find(choice => choice.length <= width) ?? choices[choices.length - 1] ?? ''
}

export function plain(key: string, text: string, style: Style = {}): Line {
  return { key, segments: [{ text, ...style }] }
}

export function prose(key: string, text: string, style: Style = {}, indent = 0): Line {
  const line: Line = { key, segments: [{ text, ...style }], wrap: true }
  if (indent > 0) line.indent = indent
  return line
}

export type Style = Omit<Segment, 'text'>

export function pair(key: string, left: string, right: string, width: number, rightStyle: Style = {}, leftStyle: Style = {}): Line {
  const text = spread(left, right, width)
  return {
    key,
    segments: [
      { text: text.slice(0, text.length - right.length), ...leftStyle },
      { text: right, ...rightStyle },
    ],
  }
}

export function heading(key: string, title: string, width: number, why?: string): Line {
  const line: Line = { key, segments: [{ text: title.padEnd(why === undefined ? width : width - WHY_COLUMNS), bold: true }] }
  if (why !== undefined) line.why = why
  return line
}

export function meter(key: string, label: string, fraction: number, value: string, width: number, color: ThemeKey, mark: number | null = null): Line {
  const labelled = label.length > 0 ? `${label} ` : ''
  const size = width - labelled.length - value.length - 1
  const cells = bar(fraction, size)
  const at = mark === null || mark <= 0 || mark >= 1 ? -1 : Math.floor(mark * size) - cells.filled.length
  const track: Segment[] =
    at < 0
      ? [{ text: cells.track, dim: true }]
      : [{ text: cells.track.slice(0, at), dim: true }, { text: MARK }, { text: cells.track.slice(at + 1), dim: true }]
  return { key, segments: [{ text: labelled }, { text: cells.filled, color }, ...track, { text: ` ${value}` }] }
}

export function contextHead(key: string, title: string, used: number | null, window: number, full: number | null, width: number, accent: Style = {}): Line {
  const amount = used === null ? tokens(window) : `${tokens(used)} / ${tokens(window)}`
  const note = full === null ? '' : ` · ${tokens(full)} model`
  const shownNote = title.length + 1 + amount.length + note.length <= width ? note : ''
  const line = pair(key, title, `${amount}${shownNote}`, width, {}, { bold: true })
  const left = line.segments[0]!
  const right: Segment[] = used === null ? [{ text: amount }] : [{ text: tokens(used), ...accent }, { text: ` / ${tokens(window)}` }]
  return { key, segments: [left, ...right, ...(shownNote === '' ? [] : [{ text: shownNote, dim: true as const }])] }
}

export function sameModel(a: string | null, b: string | null): boolean {
  return a !== null && b !== null && modelName(a) === modelName(b)
}

export function ownBreakdown(local: Pick<Local, 'breakdown' | 'model'>): LiveContext | null {
  const breakdown = local.breakdown
  return breakdown !== null && breakdown.max_tokens > 0 && sameModel(breakdown.model, local.model) ? breakdown : null
}

export function modelWindowBeyond(local: Pick<Local, 'measure' | 'model'>, model: string, window: number): number | null {
  const full = local.measure?.contextWindow ?? null
  return full !== null && full > window && sameModel(model, local.model) ? full : null
}

export function contextGauge(local: Local): ContextGauge | null {
  const measure = local.measure
  if (measure === null) return null
  const breakdown = ownBreakdown(local)
  if (breakdown === null) {
    return { tokens: measure.contextTokens, window: measure.contextWindow, percent: measure.contextPercent, modelWindow: null, threshold: null }
  }
  const window = breakdown.max_tokens
  return {
    tokens: measure.contextTokens,
    window,
    percent: measure.contextTokens === null ? null : (measure.contextTokens / window) * 100,
    modelWindow: modelWindowBeyond(local, breakdown.model, window),
    threshold: breakdown.auto_compact_threshold,
  }
}

export function notice(input: Pick<ScreenInput, 'daemon'>, width: number): Line[] {
  const { status, detail } = input.daemon
  if (status === 'ok') return []
  if (status === 'starting') return [prose('daemon', 'Starting the claude-plus daemon…', { dim: true })]
  if (status === 'rebuilding') return [prose('daemon', 'Rebuilding the claude-plus index… numbers return in a moment', { dim: true })]
  if (status === 'missing') {
    return [prose('daemon', 'claude-plus is not installed yet. Install it, or set its path in the plugin options', { color: 'warning' })]
  }
  if (status === 'outdated') return [prose('daemon', 'Restarting the out-of-date claude-plus daemon…', { dim: true })]
  if (status === 'behind') {
    const whole = 'The claude-plus daemon is older than this pane · update the claude-plus binary'
    if (whole.length <= width) return [prose('daemon', whole, { color: 'warning' })]
    return [prose('daemon', 'The claude-plus daemon is older than this pane', { color: 'warning' }), prose('daemon-fix', 'Update the claude-plus binary', { color: 'warning' })]
  }
  if (status === 'newer') {
    const whole = 'The claude-plus daemon is newer than this pane · update the plus plugin'
    if (whole.length <= width) return [prose('daemon', whole, { color: 'warning' })]
    return [prose('daemon', 'The claude-plus daemon is newer than this pane', { color: 'warning' }), prose('daemon-fix', 'Update the plus plugin', { color: 'warning' })]
  }
  if (status === 'unreachable') {
    const whole = 'The daemon is not answering · run claude-plus daemon --ensure'
    if (whole.length <= width) return [prose('daemon', whole, { color: 'warning' })]
    return [prose('daemon', 'The daemon is not answering', { color: 'warning' }), prose('daemon-fix', 'Run claude-plus daemon --ensure', { color: 'warning' })]
  }
  if (status === 'stopped') {
    const whole = 'The claude-plus daemon is stopped · run /plus to start it'
    if (whole.length <= width) return [prose('daemon', whole, { color: 'warning' })]
    return [prose('daemon', 'The claude-plus daemon is stopped', { color: 'warning' }), prose('daemon-fix', 'Run /plus to start it', { color: 'warning' })]
  }
  return [prose('daemon', `The daemon could not start: ${detail ?? 'no reason given'}`, { color: 'error' })]
}

function headline(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const sources = { ...input, snapshot }
  const id = freshest(sources, snapshot?.live.model ?? null, input.local.model)
  const effort = freshest(sources, snapshot?.live.effort ?? null, input.local.effort)
  const name = id === null ? 'Claude' : modelName(id)
  const left = effort === null ? name : `${name} · ${effort}`
  const cost = costOf(sources)
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
    const short = unpriced === 1 ? 'unpriced request' : 'unpriced requests'
    lines.push(prose('unpriced', fitting(width, `${unpriced} ${noun} not priced`, `${unpriced} ${short}`), { dim: true }))
  }
  return lines
}

function context(input: ScreenInput, width: number): Line[] {
  const gauge = contextGauge(input.local)
  if (gauge === null || gauge.percent === null) {
    return [plain('context', 'Context', { bold: true }), prose('context-empty', 'Context shows after your first prompt', { dim: true })]
  }
  const used = shown(input.motions, 'context', gauge.percent, input.nowMs)
  const color: ThemeKey = used >= CONTEXT_WARNING ? 'warning' : 'text'
  const mark = gauge.threshold === null ? null : gauge.threshold / gauge.window
  return [
    contextHead('context', 'Context', gauge.tokens, gauge.window, gauge.modelWindow, width),
    meter('context-bar', '', used / 100, percent(used).padStart(4), width, color, mark),
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
  const left = `${title}${glyph.text} ${state} (${ttl})`
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
    const price = approx(forecast.cold_usd)
    const whole = `Cache went cold while you were away, rewarming will cost ${price}`
    if (whole.length <= width) {
      lines.push(prose('cache-note', whole))
    } else {
      lines.push(prose('cache-note', 'Cache went cold while you were away'))
      lines.push(pair('cache-rewarm', '  Rewarming will cost', price, width, {}, { dim: true }))
    }
  } else if (live !== undefined && live.saved_usd >= 0.01) {
    const saved = `Cache hits saved you ${money(live.saved_usd)}`
    lines.push(prose('cache-note', fitting(width, `${saved} this session`, saved), { color: 'success' }))
  }
  if (live !== undefined && live.hit_ratio !== null) {
    const hit = shown(input.motions, 'hit', live.hit_ratio * 100, input.nowMs)
    lines.push(pair('cache-hits', '  Hit ratio', percent(hit), width, {}, { dim: true }))
  }
  if (live !== undefined && live.rebuilds > 0) {
    const value = `${live.rebuilds} · ${approx(live.rebuild_usd)}`
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
    prefix_changed: 'the MCP tools, CLAUDE.md, skills or agents changed',
    unknown: 'no cause could be told from the transcript',
  }
  return `The last rebuild, ${duration(agoMs)} ago, wrote the prompt again because ${reasons[cause] ?? reasons.unknown}.`
}

function forecast(input: ScreenInput, snapshot: Snapshot | null, width: number): Line[] {
  const next = snapshot?.live.forecast ?? null
  if (next === null || (next.warm_usd === null && next.cold_usd === null)) return []
  const warm = cacheClock(snapshot, input.local, input.nowMs).state === 'warm'
  const cost = (usd: number | null, pct: number | null) =>
    usd === null ? 'price unknown' : pct === null ? approx(usd) : `${approx(usd)} · ${finePercent(pct)} of 5h`
  const warmCost = cost(next.warm_usd, next.warm_pct_5h)
  const coldCost = cost(next.cold_usd, next.cold_pct_5h)
  const current = currentModel(next.model, warm ? warmCost : coldCost, width)
  const later = warm ? coldCost : warmCost
  const when = warm ? fitting(width - later.length - 1, '    if the cache expires', '    if it expires') : '    then'
  const after = pair('next-after', when, later, width, { dim: true }, { dim: true })
  const others = next.lineup.filter(entry => entry.model !== next.model)
  const above = next.lineup.findIndex(entry => entry.model === next.model)
  const switched = (entry: ModelCost) =>
    pair(`next-${entry.model}`, `  ${modelName(entry.model)}`, `${approx(entry.cold_usd)}, then ${approx(entry.warm_usd)}`, width, {}, { dim: true })
  const lines: Line[] = [
    forecastHeading(warm, width),
    ...others.slice(0, Math.max(0, above)).map(switched),
    current,
    after,
    ...others.slice(Math.max(0, above)).map(switched),
  ]
  if (input.expanded.includes('forecast')) {
    lines.push(
      prose(
        'explain-forecast',
        `Your ${tokens(next.prefix_tokens)} token prompt is read from the cache when warm and written again when cold. ` +
          `Each model keeps its own cache, so switching starts cold. ` +
          `${tokens(next.new_tokens)} new and ${tokens(next.output_tokens)} output tokens are typical of your last ${next.samples} requests.`,
        { dim: true },
      ),
    )
  }
  return lines
}

function forecastHeading(warm: boolean, width: number): Line {
  const title = 'Next prompt'
  const room = width - WHY_COLUMNS
  const state = ` · cache ${warm ? 'warm' : 'cold'}`
  const shown = title.length + state.length <= room ? state : ''
  return {
    key: 'next',
    why: 'forecast',
    segments: [
      { text: title, bold: true },
      { text: shown.padEnd(room - title.length), dim: true },
    ],
  }
}

function currentModel(model: string, value: string, width: number): Line {
  const marker = '● '
  const [label, price] = pair('next-current', `${marker}${modelName(model)}`, value, width).segments
  return {
    key: 'next-current',
    segments: [
      { text: marker, color: 'claude' },
      { text: (label?.text ?? '').slice(marker.length), bold: true },
      { text: price?.text ?? '', bold: true },
    ],
  }
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
  const windows = limitsOf({ ...input, snapshot })
  if (windows.length === 0) return []
  const lines: Line[] = [heading('limits', 'Plan limits', width, 'limits')]
  const room = width - DETAIL_INDENT
  const details = windows.map(limit => limitDetail(limit, input.nowMs))
  const compact = details.some(detail => detail !== null && detail.long.length > room)
  windows.forEach((limit, index) => {
    const used = shown(input.motions, `limit:${limit.window}`, limit.used_pct, input.nowMs)
    const color: ThemeKey = used >= LIMIT_WARNING ? 'warning' : 'text'
    lines.push(meter(`limit-${limit.window}`, `  ${windowName(limit.window).padEnd(5)}`, used / 100, percent(used).padStart(4), width, color))
    const detail = details[index]
    if (detail === undefined || detail === null) return
    lines.push({
      key: `limit-${limit.window}-detail`,
      segments: [{ text: (compact ? detail.short : detail.long).padEnd(room), ...(detail.warning ? { color: 'warning' } : { dim: true }) }],
      indent: DETAIL_INDENT,
    })
  })
  const five = windows.find(limit => limit.window === 'five_hour')
  if (snapshot !== null && five !== undefined && five.calibration.pct_per_usd === null) {
    const learning = 'Learning how your plan counts tokens…'
    const count = `${five.calibration.pairs} of ${five.calibration.needed} readings ✻`
    if (learning.length + 1 + count.length <= width) {
      lines.push(prose('limits-learning', `${learning} ${count}`, { dim: true }))
    } else {
      lines.push(prose('limits-learning', learning, { dim: true }), prose('limits-learning-count', count, { dim: true }))
    }
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

type Detail = { long: string; short: string; warning: boolean }

function limitDetail(limit: LimitWindow, nowMs: number): Detail | null {
  const resets = limit.resets_at_ms === null ? null : duration(Math.max(0, limit.resets_at_ms - nowMs))
  const pace = limit.pace
  const full = pace?.full_at_ms == null ? null : duration(Math.max(0, pace.full_at_ms - nowMs))
  const projected = full !== null || pace?.projected_pct == null ? null : percent(pace.projected_pct)
  const joined = (parts: (string | null)[]) => parts.filter((part): part is string => part !== null).join(' · ')
  const long = joined([resets && `resets in ${resets}`, full && `full in ${full} at this pace`, projected && `on pace for ${projected}`])
  if (long === '') return null
  return { long, short: joined([resets && `resets ${resets}`, full && `full in ${full}`, projected && `pace ${projected}`]), warning: full !== null }
}
