import { atom, read, update } from 'claude-code'
import type { EngineInterface, JsonValue, PluginOptions, Register, Timer } from 'claude-code'

import type { Daemon, Local, Motion, PaneState, PlusView, Snapshot } from '../types'
import type { Io, PlusEvent, Reply } from './client'
import { Outbox, eventId, eventsBody, fetchSnapshot, measureOf, send, socketPath } from './client'
import {
  EMPTY_LOCAL,
  easables,
  ensureError,
  ensureOutcome,
  retarget,
  screen,
  settled,
  statusText,
  stepOf,
  tickPeriod,
  toggled,
} from './state'
import { Live } from './views/live'

export const PANE_ID = 'plus'
export const PANE_TITLE = 'claude-plus'
export const POLL_MS = 3000
export const ENSURE_TIMEOUT_MS = 10_000
export const FULLSCREEN_KEY = 'fullscreen'
export const SERVICE_UNAVAILABLE = 503
export const STUCK_POST_MS = 30_000

const view = atom({ plugin: 'plus', key: 'view' } as const, 'live' as PlusView)
const snapshot = atom({ plugin: 'plus', key: 'snapshot' } as const, null as Snapshot | null, { shape: 'snapshot-1' })
const local = atom({ plugin: 'plus', key: 'local' } as const, EMPTY_LOCAL as Local, { shape: 'local-1' })
const pane = atom({ plugin: 'plus', key: 'pane' } as const, { session: null, asked: false } as PaneState)
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
  posting: { sinceMs: number } | null
  idPrefix: string | null
  sequence: number
  polledAtMs: number
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
    posting: null,
    idPrefix: null,
    sequence: 0,
    polledAtMs: -Infinity,
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

function ensureTimers($: EngineInterface): void {
  if (runtime.timer !== null) return
  runtime.timer = $.clock.after(0, () => void tick($))
}

async function schedule($: EngineInterface): Promise<void> {
  const period = tickPeriod({
    animations: runtime.animations,
    motions: await read($, motions),
    nowMs: await read($, now),
    snapshot: await read($, snapshot),
    local: await read($, local),
  })
  runtime.timer = $.clock.after(period, () => void tick($))
}

async function tick($: EngineInterface): Promise<void> {
  try {
    const at = await $.clock.now()
    await update($, now, () => at)
    if (runtime.outbox.size > 0) await flush($)
    if (at - runtime.polledAtMs >= POLL_MS) await poll($)
    await settle($)
    await showStatus($)
  } finally {
    await schedule($)
  }
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
    await update($, motions, () => ({}))
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
  if (reply.kind === 'failed') {
    if (status === 'ok' || status === 'rebuilding') await update($, daemon, (): Daemon => ({ status: 'unreachable', detail: reply.reason }))
    return
  }
  const parsed = reply.snapshot
  if (parsed.session !== id) return
  if (status !== 'ok') await update($, daemon, (): Daemon => ({ status: 'ok', detail: null }))
  await animate($, async () => {
    await update($, snapshot, () => parsed)
  })
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
  if (!reply.ok && reply.late === true) void sent.settled.then(done)
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

function ensureDaemon($: EngineInterface): Promise<void> {
  runtime.ensuring ??= (async () => {
    const outcome = await $.process.run([runtime.binary, 'daemon', '--ensure'], { timeoutMs: ENSURE_TIMEOUT_MS }).then(
      result => ensureOutcome(result.exitCode, result.stderr),
      ensureError,
    )
    await update($, daemon, () => outcome)
    if (outcome.status === 'ok') {
      await flush($)
      await poll($)
    } else {
      runtime.ensuring = null
    }
  })()
  return runtime.ensuring
}

async function open($: EngineInterface, fullscreen: boolean): Promise<string> {
  const opened = await $.ui.open({ id: PANE_ID, title: PANE_TITLE })
  if (!opened.isPlaced) return `✻ claude-plus will show once there is room: ${opened.reason}`
  return fullscreen ? '✻ claude-plus is open beside the conversation' : '✻ claude-plus is open above the prompt'
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
    void ensureDaemon($)
    if ((await read($, pane)).session === id) return next(e)
    await update($, pane, (): PaneState => ({ session: id, asked: false }))
    void forward($, 'session.start', { cwd: e.cwd, surface: e.surface, interactive: e.isInteractive })
    if (e.isInteractive && (await $.store.get(FULLSCREEN_KEY)) === true) void open($, true)
    return next(e)
  })

  on('command.run', { command: 'plus' }, async ($, e) => {
    ensureTimers($)
    void ensureDaemon($)
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
    void forward($, 'session.measure', {
      model,
      context: { tokens: e.context.tokens ?? null, window: e.context.window, percent: e.context.percent ?? null },
      rateLimits: e.rateLimits.map(limit => ({ kind: limit.kind, percentUsed: limit.percentUsed, resetsAt: limit.resetsAt ?? null })),
      cost_usd: e.cost?.usd ?? null,
      changed: [...e.changed],
    })
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
      void forward($, 'turn.step', {
        sent_ms: sentAt,
        turnId: e.turnId,
        index: e.index,
        agentId: e.agentId ?? null,
        model: e.model,
        effort: e.effort ?? null,
        stopReason: result.stopReason,
        usage: { ...usage },
      })
    }
    return result
  })

  on('turn.complete', async ($, e, next) => {
    ensureTimers($)
    await follow($)
    if (e.agentId === undefined) {
      const turns = await $.session.turns()
      await update($, local, known => ({ ...known, working: false, turns }))
    }
    void forward($, 'turn.complete', { turnId: e.turnId, agentId: e.agentId ?? null, reason: e.reason, usage: e.usage === undefined ? null : { ...e.usage } })
    runtime.polledAtMs = -Infinity
    return next(e)
  })

  on('session.compact', async ($, e, next) => {
    const result = await next(e)
    void forward($, 'session.compact', {
      trigger: e.trigger,
      agentId: e.agentId ?? null,
      skipped: result.skip ?? null,
      tokensBefore: result.skip === undefined ? (result.tokensBefore ?? null) : null,
      tokensAfter: result.skip === undefined ? (result.tokensAfter ?? null) : null,
    })
    return result
  })

  on('session.end', async ($, e, next) => {
    void forward($, 'session.end', { reason: e.reason }, e.sessionId)
    return next(e)
  })

  on('ui.render', { component: 'Pane', requestId: PANE_ID }, async ($, e) => {
    if (e.props.placement === 'inline' || e.viewport?.isFullscreen === false) $.clock.after(0, () => void closeUnasked($).catch(() => undefined))
    const nowMs = await read($, now)
    const current = await read($, view)
    const lines = screen({
      width: e.props.bodyColumns,
      nowMs,
      animations: runtime.animations,
      snapshot: await read($, snapshot),
      local: await read($, local),
      daemon: await read($, daemon),
      expanded: await read($, expanded),
      motions: await read($, motions),
    })
    return (
      <Live
        ui={$.ui.resolve(e)}
        width={e.props.bodyColumns}
        lines={lines}
        tabs={[{ key: 'tab-live', hotkey: '1', label: 'Live', isActive: current === 'live', onPress: () => void update($, view, () => 'live') }]}
        onWhy={key => void update($, expanded, list => toggled(list, key))}
      />
    )
  })
}
