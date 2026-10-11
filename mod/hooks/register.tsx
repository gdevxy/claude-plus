import { atom, read, update } from 'claude-code'
import type { EngineInterface, JsonValue, PluginOptions, Register, Timer } from 'claude-code'

import type { Chart, ContextReport, Daemon, DaemonStatus, Local, Motion, PaneState, PlusView, Reports, Snapshot } from '../types'
import type { Io, PlusEvent, Reply } from './client'
import { Outbox, breakdownOf, eventId, eventsBody, fetchContext, fetchHistory, fetchSnapshot, measureOf, send, socketPath, usageData } from './client'
import { contextScreen } from './context'
import { historyScreen, metricLabel, nextMetric, nextRange, rangeLabel, shownMetric } from './history'
import { paletteOf } from './palette'
import {
  EMPTY_LOCAL,
  TICK_MS,
  cacheClock,
  current,
  easables,
  ensureError,
  ensureOutcome,
  retarget,
  screen,
  settled,
  statusText,
  stepOf,
  nextTickDelay,
  tickPeriod,
  toggled,
} from './state'
import { PaneBody } from './views/pane'

export const PANE_ID = 'plus'
export const PANE_TITLE = 'claude-plus'
export const POLL_MS = 3000
export const ENSURE_TIMEOUT_MS = 10_000
export const FULLSCREEN_KEY = 'fullscreen'
export const SERVICE_UNAVAILABLE = 503
export const STUCK_POST_MS = 30_000
export const ENSURE_RETRY_MS = 30_000
export const CONTEXT_POLL_MS = 10_000
export const USAGE_GAP_MS = 60_000
export const USAGE_IDLE_GAP_MS = 600_000
export const USAGE_COLUMNS = 40
export const HISTORY_POLL_MS = 30_000
export const THEME_KEY = 'theme'

const view = atom({ plugin: 'plus', key: 'view' } as const, 'live' as PlusView)
const snapshot = atom({ plugin: 'plus', key: 'snapshot' } as const, null as Snapshot | null, { shape: 'snapshot-1' })
const context = atom({ plugin: 'plus', key: 'context' } as const, null as ContextReport | null, { shape: 'context-1' })
const history = atom({ plugin: 'plus', key: 'history' } as const, {} as Reports, { shape: 'history-1' })
const chart = atom({ plugin: 'plus', key: 'chart' } as const, { range: '7d', metric: 'cost' } as Chart)
const theme = atom({ plugin: 'plus', key: 'theme' } as const, 'auto')
const local = atom({ plugin: 'plus', key: 'local' } as const, EMPTY_LOCAL as Local, { shape: 'local-2' })
const pane = atom({ plugin: 'plus', key: 'pane' } as const, { session: null, asked: false, unasked: 'settled' } as PaneState)
const daemon = atom({ plugin: 'plus', key: 'daemon' } as const, { status: 'starting', detail: null } as Daemon)
const now = atom({ plugin: 'plus', key: 'now' } as const, 0)
const expanded = atom({ plugin: 'plus', key: 'expanded' } as const, [] as string[])
const motions = atom({ plugin: 'plus', key: 'motions' } as const, {} as Record<string, Motion>)

type Runtime = {
  binary: string
  animations: boolean
  outbox: Outbox
  timer: Timer | null
  socket: string | null | undefined
  ensuring: Promise<void> | null
  ensuredAtMs: number
  replaceTried: boolean
  posting: { sinceMs: number } | null
  idPrefix: string | null
  sequence: number
  polledAtMs: number
  contextPolledAtMs: number
  historyPolledAtMs: number
  historyRunning: boolean
  usageDue: boolean
  usageRunning: boolean
  usageAtMs: number
  shownStatus: string | undefined
}

function fresh(options: PluginOptions): Runtime {
  return {
    binary: typeof options.binaryPath === 'string' && options.binaryPath !== '' ? options.binaryPath : 'claude-plus',
    animations: options.animations !== false,
    outbox: new Outbox(),
    timer: null,
    socket: undefined,
    ensuring: null,
    ensuredAtMs: -Infinity,
    replaceTried: false,
    posting: null,
    idPrefix: null,
    sequence: 0,
    polledAtMs: -Infinity,
    contextPolledAtMs: -Infinity,
    historyPolledAtMs: -Infinity,
    historyRunning: false,
    usageDue: false,
    usageRunning: false,
    usageAtMs: -Infinity,
    shownStatus: undefined,
  }
}

let runtime = fresh({})

function io($: EngineInterface): Io {
  return {
    fetch: (url, init) => $.http.fetch(url, init),
    sleep: (ms, sleepOptions) => $.clock.sleep(ms, sleepOptions),
  }
}

async function socketOf($: EngineInterface): Promise<string | null> {
  if (runtime.socket === undefined) runtime.socket = socketPath(await $.env.get('HOME'), await $.env.get('CLAUDE_PLUS_HOME'))
  return runtime.socket
}

function quietly(work: Promise<unknown>): void {
  work.catch(() => undefined)
}

function ensureTimers($: EngineInterface): void {
  if (runtime.timer !== null) return
  arm($, 0)
}

function arm($: EngineInterface, delayMs: number): void {
  try {
    runtime.timer = $.clock.after(delayMs, () => quietly(tick($)))
  } catch {
    runtime.timer = null
  }
}

async function delayToNextTick($: EngineInterface): Promise<number> {
  try {
    const input = {
      animations: runtime.animations,
      motions: await read($, motions),
      nowMs: await $.clock.now(),
      snapshot: await read($, snapshot),
      local: await read($, local),
    }
    return nextTickDelay(tickPeriod(input), cacheClock(current(input), input.local, input.nowMs).expiresAtMs ?? 0, input.nowMs)
  } catch {
    return TICK_MS
  }
}

async function tick($: EngineInterface): Promise<void> {
  try {
    const at = await $.clock.now()
    await update($, now, () => at)
    await settle($)
    await showStatus($)
    if (runtime.outbox.size > 0) await flush($)
    if (at - runtime.polledAtMs >= POLL_MS) await poll($)
    const shownView = await read($, view)
    const usageWanted = runtime.usageDue && !runtime.usageRunning
    const contextWanted = shownView !== 'live' && at - runtime.contextPolledAtMs >= CONTEXT_POLL_MS
    const historyWanted = shownView === 'history' && !runtime.historyRunning && at - runtime.historyPolledAtMs >= HISTORY_POLL_MS
    if ((usageWanted || contextWanted || historyWanted) && (await paneShown($))) {
      const usageGap = shownView === 'context' ? USAGE_GAP_MS : USAGE_IDLE_GAP_MS
      if (usageWanted && at - runtime.usageAtMs >= usageGap) quietly(collectUsage($))
      if (contextWanted) await pollContext($)
      if (historyWanted) quietly(pollHistory($))
    }
    if (needsEnsure((await read($, daemon)).status) && at - runtime.ensuredAtMs >= ENSURE_RETRY_MS) quietly(ensureDaemon($))
  } catch {
    return
  } finally {
    arm($, await delayToNextTick($))
  }
}

async function paneShown($: EngineInterface): Promise<boolean> {
  const panes = await $.ui.panes().catch(() => [])
  return panes.some(pane => pane.id === PANE_ID && pane.isPlaced && pane.isShown)
}

async function settle($: EngineInterface): Promise<void> {
  const at = await read($, now)
  const running = await read($, motions)
  const kept = settled(running, at)
  if (Object.keys(kept).length !== Object.keys(running).length) await update($, motions, () => kept)
}

async function sources($: EngineInterface) {
  return { snapshot: await read($, snapshot), local: await read($, local), daemon: await read($, daemon), nowMs: await read($, now) }
}

async function showStatus($: EngineInterface): Promise<void> {
  const text = statusText(await sources($))
  if (text === runtime.shownStatus) return
  runtime.shownStatus = text
  $.ui.status(text)
}

async function animate($: EngineInterface, change: () => Promise<void>): Promise<void> {
  const before = easables(await sources($))
  await change()
  const after = easables(await sources($))
  const at = await $.clock.now()
  await update($, motions, running => retarget(running, before, after, at, runtime.animations))
  await update($, now, () => at)
  await showStatus($)
}

async function follow($: EngineInterface): Promise<string> {
  const id = await $.session.id()
  const known = await read($, local)
  if (known.session !== id) {
    await update($, local, () => ({ ...EMPTY_LOCAL, session: id }))
    await update($, snapshot, () => null)
    await update($, context, () => null)
    await update($, motions, () => ({}))
    runtime.usageAtMs = -Infinity
    runtime.contextPolledAtMs = -Infinity
  }
  return id
}

async function poll($: EngineInterface): Promise<void> {
  runtime.polledAtMs = await $.clock.now()
  const id = await follow($)
  const path = await socketOf($)
  if (path === null) return
  const reply = await fetchSnapshot(io($), path, id)
  const status = (await read($, daemon)).status
  if (reply.kind === 'rebuilding') {
    await update($, daemon, (): Daemon => ({ status: 'rebuilding', detail: null }))
    return
  }
  if (reply.kind === 'outdated') {
    const next: DaemonStatus = reply.newer === 'daemon' ? 'newer' : runtime.replaceTried ? 'behind' : 'outdated'
    if (status !== next) await update($, daemon, (): Daemon => ({ status: next, detail: null }))
    if (next === 'outdated') {
      runtime.replaceTried = true
      quietly((runtime.ensuring ?? Promise.resolve()).then(() => ensureDaemon($)))
    }
    return
  }
  if (reply.kind === 'failed') {
    if (status === 'ok' || status === 'rebuilding') await update($, daemon, (): Daemon => ({ status: 'unreachable', detail: reply.reason }))
    return
  }
  const parsed = reply.snapshot
  if (parsed.session !== id) return
  runtime.replaceTried = false
  if (status !== 'ok') await update($, daemon, (): Daemon => ({ status: 'ok', detail: null }))
  await animate($, async () => {
    await update($, snapshot, () => parsed)
  })
}

async function pollContext($: EngineInterface): Promise<void> {
  runtime.contextPolledAtMs = await $.clock.now()
  const id = await follow($)
  const path = await socketOf($)
  if (path === null) return
  const reply = await fetchContext(io($), path, id)
  if (reply.kind !== 'context' || reply.report.session !== id) return
  await update($, context, () => reply.report)
}

async function pollHistory($: EngineInterface): Promise<void> {
  runtime.historyRunning = true
  try {
    runtime.historyPolledAtMs = await $.clock.now()
    await readTheme($)
    const path = await socketOf($)
    if (path === null) return
    const range = (await read($, chart)).range
    const reply = await fetchHistory(io($), path, range)
    if (reply.kind !== 'history' || reply.report.range !== range) return
    await update($, history, known => ({ ...known, [range]: reply.report }))
  } finally {
    runtime.historyRunning = false
  }
}

async function readTheme($: EngineInterface): Promise<void> {
  const rows = await $.config.list().catch(() => [])
  const value = rows.find(row => row.key === THEME_KEY)?.value
  if (typeof value === 'string') await update($, theme, () => value)
}

async function cycleRange($: EngineInterface): Promise<void> {
  await update($, chart, known => ({ ...known, range: nextRange(known.range) }))
  runtime.historyPolledAtMs = -Infinity
  ensureTimers($)
}

async function cycleMetric($: EngineInterface): Promise<void> {
  const reports = await read($, history)
  await update($, chart, known => ({ ...known, metric: nextMetric(known.metric, reports[known.range] ?? null) }))
}

async function collectUsage($: EngineInterface): Promise<void> {
  runtime.usageRunning = true
  runtime.usageDue = false
  try {
    runtime.usageAtMs = await $.clock.now()
    const id = await follow($)
    const usage = await $.session.usage({ breakdown: 'full', columns: USAGE_COLUMNS }).catch(() => null)
    if (usage === null || (await follow($)) !== id) return
    const at = await $.clock.now()
    const breakdown = breakdownOf(usage, at)
    const data = usageData(usage)
    if (breakdown === null || data === null) return
    await update($, local, known => ({ ...known, breakdown }))
    runtime.contextPolledAtMs = -Infinity
    await forward($, 'session.usage', data, id)
  } finally {
    runtime.usageRunning = false
  }
}

async function showView($: EngineInterface, next: PlusView): Promise<void> {
  await update($, view, () => next)
  if (next === 'live') return
  runtime.contextPolledAtMs = -Infinity
  if (next === 'history') {
    runtime.historyPolledAtMs = -Infinity
    ensureTimers($)
    return
  }
  if ((await read($, local)).breakdown === null) runtime.usageDue = true
  ensureTimers($)
}

async function flush($: EngineInterface): Promise<void> {
  const path = await socketOf($)
  if (path === null || runtime.outbox.size === 0) return
  const at = await $.clock.now()
  if (runtime.posting !== null && at - runtime.posting.sinceMs < STUCK_POST_MS) return
  const posting = { sinceMs: at }
  runtime.posting = posting
  const events = runtime.outbox.take()
  const sent = send(io($), path, '/events', { method: 'POST', headers: { 'content-type': 'application/json' }, body: eventsBody(events) })
  const done = (reply: Reply) => {
    if (!reply.ok) runtime.outbox.putBack(events)
    if (runtime.posting === posting) runtime.posting = null
  }
  const reply = await sent.reply
  if (!reply.ok && reply.late === true) quietly(sent.settled.then(done))
  else done(reply)
}

async function forward($: EngineInterface, kind: string, data: { [key: string]: JsonValue }, session?: string): Promise<void> {
  const ts_ms = await $.clock.now()
  runtime.idPrefix ??= `${ts_ms.toString(36)}${Math.random().toString(36).slice(2, 8)}`
  runtime.sequence += 1
  const event: PlusEvent = { id: eventId(runtime.idPrefix, runtime.sequence), session: session ?? (await follow($)), kind, ts_ms, data }
  runtime.outbox.add(event)
  await flush($)
}

function needsEnsure(status: DaemonStatus): boolean {
  return status === 'unreachable'
}

function settlesOnPoll(status: DaemonStatus): boolean {
  return status === 'outdated' || status === 'behind' || status === 'newer'
}

function ensureDaemon($: EngineInterface): Promise<void> {
  runtime.ensuring ??= (async () => {
    try {
      runtime.ensuredAtMs = await $.clock.now()
      const outcome = await $.process.run([runtime.binary, 'daemon', '--ensure'], { timeoutMs: ENSURE_TIMEOUT_MS }).then(
        result => ensureOutcome(result.exitCode, result.stderr),
        ensureError,
      )
      await update($, daemon, known => (outcome.status === 'ok' && settlesOnPoll(known.status) ? known : outcome))
      if (outcome.status !== 'ok') return
      await flush($)
      await poll($)
    } finally {
      runtime.ensuring = null
    }
  })()
  return runtime.ensuring
}

async function open($: EngineInterface, fullscreen: boolean): Promise<string> {
  const opened = await $.ui.open({ id: PANE_ID, title: PANE_TITLE }).catch((error: unknown) => refusal(error))
  if (typeof opened === 'string') return `✻ claude-plus could not open: ${opened}`
  if (!opened.isPlaced) return `✻ claude-plus will show once there is room: ${opened.reason}`
  return fullscreen ? '✻ claude-plus is open beside the conversation' : '✻ claude-plus is open above the prompt'
}

function refusal(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return message.replace(/^.*\$\.ui\.open:\s*/, '')
}

async function openUnasked($: EngineInterface, fullscreen: boolean): Promise<void> {
  const known = await read($, pane)
  if (known.unasked !== 'waiting') return
  await update($, pane, (): PaneState => ({ ...known, unasked: 'settled' }))
  if (fullscreen && (await $.store.get(FULLSCREEN_KEY)) !== false) await open($, true)
}

async function closeUnasked($: EngineInterface): Promise<void> {
  if ((await read($, pane)).asked) return
  await $.store.set(FULLSCREEN_KEY, false)
  await $.ui.close({ id: PANE_ID })
}

export const register: Register = (on, options) => {
  runtime = fresh(options)

  on('session.start', async ($, e, next) => {
    ensureTimers($)
    await $.command.register({ name: 'plus', description: 'Open the claude-plus pane' })
    const id = await follow($)
    const model = await $.session.model()
    const turns = await $.session.turns()
    await update($, local, known => ({ ...known, session: id, model, turns }))
    quietly(ensureDaemon($))
    quietly(readTheme($))
    if ((await read($, pane)).session === id) return next(e)
    await update($, pane, (): PaneState => ({ session: id, asked: false, unasked: e.isInteractive ? 'waiting' : 'settled' }))
    quietly(forward($, 'session.start', { cwd: e.cwd, surface: e.surface, interactive: e.isInteractive }))
    return next(e)
  })

  on('command.run', { command: 'plus' }, async ($, e) => {
    ensureTimers($)
    runtime.replaceTried = true
    quietly(ensureDaemon($))
    await $.store.set(FULLSCREEN_KEY, e.presentation.isFullscreen)
    await update($, pane, known => ({ ...known, asked: true }))
    return { text: await open($, e.presentation.isFullscreen) }
  })

  on('turn.start', async ($, e, next) => {
    ensureTimers($)
    await follow($)
    await update($, local, known => ({ ...known, working: true }))
    return next(e)
  })

  on('session.measure', async ($, e, next) => {
    ensureTimers($)
    await follow($)
    const at = await $.clock.now()
    const model = await $.session.model()
    await animate($, async () => {
      await update($, local, known => ({ ...known, model, measure: measureOf(e, at) }))
    })
    quietly(forward($, 'session.measure', {
      model,
      context: { tokens: e.context.tokens ?? null, window: e.context.window, percent: e.context.percent ?? null },
      rateLimits: e.rateLimits.map(limit => ({ kind: limit.kind, percentUsed: limit.percentUsed, resetsAt: limit.resetsAt ?? null })),
      cost_usd: e.cost?.usd ?? null,
      changed: [...e.changed],
    }))
    return next(e)
  })

  on('turn.step', async function* ($, e, next) {
    const sentAt = await $.clock.now()
    const result = yield* next(e)
    const usage = result.usage
    if (usage !== null) {
      ensureTimers($)
      await follow($)
      const at = await $.clock.now()
      if (e.agentId === undefined) {
        const effort = e.effort === undefined ? null : String(e.effort)
        await update($, local, known => ({ ...known, model: e.model, effort, step: stepOf(usage, sentAt) }))
      }
      await update($, now, () => at)
      quietly(forward($, 'turn.step', {
        sent_ms: sentAt,
        turnId: e.turnId,
        index: e.index,
        agentId: e.agentId ?? null,
        model: e.model,
        effort: e.effort ?? null,
        stopReason: result.stopReason,
        usage: { ...usage },
      }))
    }
    return result
  })

  on('turn.complete', async ($, e, next) => {
    ensureTimers($)
    await follow($)
    if (e.agentId === undefined) {
      const turns = await $.session.turns()
      await update($, local, known => ({ ...known, working: false, turns }))
      runtime.usageDue = true
    }
    quietly(forward($, 'turn.complete', { turnId: e.turnId, agentId: e.agentId ?? null, reason: e.reason, usage: e.usage === undefined ? null : { ...e.usage } }))
    runtime.polledAtMs = -Infinity
    return next(e)
  })

  on('session.compact', async ($, e, next) => {
    const result = await next(e)
    if (e.agentId === undefined) runtime.usageDue = true
    quietly(forward($, 'session.compact', {
      trigger: e.trigger,
      agentId: e.agentId ?? null,
      skipped: result.skip ?? null,
      tokensBefore: result.skip === undefined ? (result.tokensBefore ?? null) : null,
      tokensAfter: result.skip === undefined ? (result.tokensAfter ?? null) : null,
    }))
    return result
  })

  on('config.set', { key: THEME_KEY }, async ($, e, next) => {
    const result = await next(e)
    if (result.deny === undefined && typeof result.value === 'string') {
      const value = result.value
      await update($, theme, () => value)
    }
    return result
  })

  on('session.end', async ($, e, next) => {
    quietly(forward($, 'session.end', { reason: e.reason }, e.sessionId))
    return next(e)
  })

  on('ui.render', { component: 'PromptHint' }, async ($, e, next) => {
    const fullscreen = e.viewport?.isFullscreen
    if (fullscreen !== undefined && (await read($, pane)).unasked === 'waiting') $.clock.after(0, () => quietly(openUnasked($, fullscreen)))
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE_ID }, async ($, e) => {
    if (e.props.placement === 'inline' || e.viewport?.isFullscreen === false) $.clock.after(0, () => quietly(closeUnasked($)))
    const nowMs = await read($, now)
    const shownView = await read($, view)
    const known = await read($, local)
    const status = await read($, daemon)
    const open = await read($, expanded)
    const chosen = await read($, chart)
    const reports = await read($, history)
    const lines =
      shownView === 'context'
        ? contextScreen({ width: e.props.bodyColumns, nowMs, report: await read($, context), local: known, daemon: status, expanded: open })
        : shownView === 'history'
          ? historyScreen({ width: e.props.bodyColumns, range: chosen.range, metric: chosen.metric, reports, context: await read($, context), local: known, daemon: status, expanded: open })
          : screen({
              width: e.props.bodyColumns,
              nowMs,
              animations: runtime.animations,
              snapshot: await read($, snapshot),
              local: known,
              daemon: status,
              expanded: open,
              motions: await read($, motions),
            })
    const ui = $.ui.resolve(e)
    const shown = reports[chosen.range] ?? null
    return (
      <PaneBody
        ui={ui}
        width={e.props.bodyColumns}
        lines={lines}
        palette={e.surface === 'terminal' ? paletteOf(await read($, theme)) : null}
        tabs={[
          { key: 'tab-live', hotkey: '1', label: 'Live', isActive: shownView === 'live', onPress: () => quietly(showView($, 'live')) },
          { key: 'tab-context', hotkey: '2', label: 'Context', isActive: shownView === 'context', onPress: () => quietly(showView($, 'context')) },
          { key: 'tab-history', hotkey: '3', label: 'History', isActive: shownView === 'history', onPress: () => quietly(showView($, 'history')) },
        ]}
        controls={
          shownView === 'history'
            ? [
                { key: 'range', hotkey: 'r', label: rangeLabel(chosen.range), onPress: () => quietly(cycleRange($)) },
                { key: 'metric', hotkey: 'g', label: metricLabel(shownMetric(chosen.metric, shown)), onPress: () => quietly(cycleMetric($)) },
              ]
            : []
        }
        onToggle={key => quietly(update($, expanded, list => toggled(list, key)))}
      />
    )
  })
}
