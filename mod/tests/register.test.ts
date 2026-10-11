import { describe, expect, mock, test } from 'claude-code/testing'
import type { Engine, FoundElement } from 'claude-code/testing'
import type { HttpInit, On } from 'claude-code'

import type { Snapshot } from '../types'
import { MINUTE_MS, NOW_MS, snapshot } from './fixtures'
import { paintTree, trimmed } from './paint'

type Wiring = {
  session: string
  served: Snapshot | null
  serving: 'current' | 'older' | 'newer'
  lag: (() => Promise<void>) | null
  openRefusal: string | null
  resets: number
  ensureCures: boolean
  silent: boolean
  holding: boolean
  held: (() => void)[]
  down: boolean
  rebuilding: boolean
  exitCode: number
  fetched: { url: string; init?: HttpInit }[]
  delivered: { url: string; init?: HttpInit }[]
  ran: (readonly string[])[]
  opened: string[]
  closed: string[]
  statuses: (string | undefined)[]
}

const MEASURE = {
  context: { tokens: 46_000, window: 200_000, percent: 23 },
  rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAt: new Date(NOW_MS + 130 * MINUTE_MS).toISOString() }],
  cost: { usd: 4.18 },
  changed: ['context' as const],
}

const PANE_FRAMES: Record<number, string[]> = {
  40: [
    ' ✻ claude-plus                     Live',
    '',
    ' Opus 5.5 · high                  $4.18',
    '',
    ' Context                     46k / 200k',
    ' ███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    '',
    ' Cache ✻ warm (1h)         52:13 left ?',
    ' █████████████████████████████████▏░░░░',
    ' Cache hits saved you $3.20',
    '   Hit ratio                        93%',
    '   Rebuilds                1 · ~$0.40 ?',
    '',
    ' Next turn                            ?',
    '   if warm         ~$0.03 · <0.1% of 5h',
    '   if cold          ~$0.40 · 0.2% of 5h',
    '   on Sonnet 5.5    ~$0.20, then ~$0.02',
    '',
    ' Session',
    '   Tokens in / out             2M / 61k',
    '   Turns · subagents             14 · 2',
    '',
    ' Plan limits                          ?',
    '   5h    █████████████▌░░░░░░░░░░░  54%',
    '         resets 2h 9m · pace 81%',
    '   7d    ███████▊░░░░░░░░░░░░░░░░░  31%',
    '         resets 2d 23h',
  ],
  60: [
    ' ✻ claude-plus                                         Live',
    '',
    ' Opus 5.5 · high                                      $4.18',
    '',
    ' Context                                         46k / 200k',
    ' ████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    '',
    ' Cache ✻ warm (1h)                             52:13 left ?',
    ' ██████████████████████████████████████████████████▌░░░░░░░',
    ' Cache hits saved you $3.20 this session',
    '   Hit ratio                                            93%',
    '   Rebuilds                                    1 · ~$0.40 ?',
    '',
    ' Next turn                                                ?',
    '   if warm                             ~$0.03 · <0.1% of 5h',
    '   if cold                              ~$0.40 · 0.2% of 5h',
    '   on Sonnet 5.5                        ~$0.20, then ~$0.02',
    '',
    ' Session',
    '   Tokens in / out                                 2M / 61k',
    '   Turns · subagents                                 14 · 2',
    '',
    ' Plan limits                                              ?',
    '   5h    ████████████████████████▎░░░░░░░░░░░░░░░░░░░░  54%',
    '         resets in 2h 9m · on pace for 81%',
    '   7d    ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
    '         resets in 2d 23h',
  ],
  100: [
    ' ✻ claude-plus                                                                                 Live',
    '',
    ' Opus 5.5 · high                                                                              $4.18',
    '',
    ' Context                                                                                 46k / 200k',
    ' █████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    '',
    ' Cache ✻ warm (1h)                                                                     52:13 left ?',
    ' █████████████████████████████████████████████████████████████████████████████████████▎░░░░░░░░░░░░',
    ' Cache hits saved you $3.20 this session',
    '   Hit ratio                                                                                    93%',
    '   Rebuilds                                                                            1 · ~$0.40 ?',
    '',
    ' Next turn                                                                                        ?',
    '   if warm                                                                     ~$0.03 · <0.1% of 5h',
    '   if cold                                                                      ~$0.40 · 0.2% of 5h',
    '   on Sonnet 5.5                                                                ~$0.20, then ~$0.02',
    '',
    ' Session',
    '   Tokens in / out                                                                         2M / 61k',
    '   Turns · subagents                                                                         14 · 2',
    '',
    ' Plan limits                                                                                      ?',
    '   5h    █████████████████████████████████████████████▉░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  54%',
    '         resets in 2h 9m · on pace for 81%',
    '   7d    ██████████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
    '         resets in 2d 23h',
  ],
}

function wire(on: On, overrides: Partial<Wiring> = {}, stored: Record<string, unknown> = { fullscreen: true }): Wiring {
  const wiring: Wiring = { session: 's1', served: snapshot(), serving: 'current', lag: null, openRefusal: null, resets: 0, ensureCures: true, silent: false, holding: false, held: [], down: false, rebuilding: false, exitCode: 0, fetched: [], delivered: [], ran: [], opened: [], closed: [], statuses: [], ...overrides }
  mock.env(on, { HOME: '/Users/me' })
  mock.store(on, stored)
  on('session.id', () => ({ value: wiring.session }))
  on('session.model', () => ({ value: 'claude-opus-5-5' }))
  on('session.turns', () => ({ value: 14 }))
  on('command.register', ($, e) => ({ value: { command: e.name } }))
  on('ui.open', ($, e) => {
    if (wiring.openRefusal !== null) return { deny: wiring.openRefusal }
    wiring.opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.close', ($, e) => {
    wiring.closed.push(e.id)
    return { value: undefined }
  })
  on('ui.status', ($, e) => {
    wiring.statuses.push(e.text)
    return { value: undefined }
  })
  on('process.run', ($, e) => {
    wiring.ran.push(e.argv)
    if (wiring.serving === 'older' && wiring.ensureCures) wiring.serving = 'current'
    return { value: { exitCode: wiring.exitCode, stdout: '', stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('http.fetch', async ($, e) => {
    wiring.fetched.push(e)
    if (wiring.silent) await new Promise(() => undefined)
    if (wiring.lag !== null && e.url.includes('/snapshot')) await wiring.lag()
    if (wiring.holding && e.url.endsWith('/events')) await new Promise<void>(resolve => wiring.held.push(resolve))
    if (wiring.down) throw new Error('ECONNREFUSED')
    if (wiring.resets > 0) {
      wiring.resets -= 1
      throw new Error('ECONNRESET')
    }
    if (e.url.endsWith('/events')) wiring.delivered.push(e)
    if (wiring.rebuilding && e.url.includes('/snapshot')) return { value: { status: 503, ok: false, headers: {}, text: '{"error":"rebuilding"}' } }
    const body = e.url.includes('/snapshot') ? JSON.stringify(servedBody(wiring)) : '{"accepted":1,"rejected":[]}'
    return { value: { status: 200, ok: true, headers: {}, text: body } }
  })
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('session.measure', ($, e) => ({ changed: e.changed }))
  on('turn.start', ($, e) => ({ turnId: e.turnId }))
  on('turn.complete', () => ({ text: '' }))
  return wiring
}

function servedBody(wiring: Wiring): unknown {
  if (wiring.served === null) return { v: 1 }
  if (wiring.serving === 'newer') return { v: 2, ...wiring.served }
  if (wiring.serving === 'older') {
    const { live: _live, ...older } = wiring.served
    return { v: 1, ...older }
  }
  return { v: 1, ...wiring.served }
}

async function start($: Engine): Promise<void> {
  await $.session.start({ cwd: '/work', surface: 'terminal', isInteractive: true })
}

async function mount($: Engine, columns: number, fullscreen = true) {
  return $.ui.mount({
    plugin: 'plus',
    surface: 'terminal',
    component: 'Pane',
    requestId: 'plus',
    props: { title: 'claude-plus', isFocused: true, bodyColumns: columns, placement: fullscreen ? 'dock' : 'inline', scroll: { offset: 0, bodyRows: 40 }, view: {} },
    viewport: { columns: fullscreen ? columns + 100 : columns + 2, rows: 50, isFullscreen: fullscreen },
  })
}

function glyph(cache: FoundElement | undefined): string {
  const row = cache?.children[0] as { children: unknown[] } | undefined
  return JSON.stringify(row?.children[1])
}

type Posted = { id?: string; kind: string }

function posted(calls: { url: string; init?: HttpInit }[]): Posted[] {
  return calls
    .filter(call => call.url.endsWith('/events'))
    .flatMap(call => (JSON.parse(call.init?.body ?? '{}') as { events: Posted[] }).events)
}

function kinds(wiring: Wiring): string[] {
  return posted(wiring.delivered).map(event => event.kind)
}

describe('plus mod', () => {
  test('a session start ensures the daemon, registers /plus and opens the pane', { options: { binaryPath: '/opt/bin/claude-plus' } }, async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.settle()
    expect(wiring.ran).toEqual([['/opt/bin/claude-plus', 'daemon', '--ensure']])
    expect(wiring.opened).toEqual(['plus'])
    expect(wiring.fetched[0]?.init?.socketPath).toBe('/Users/me/.claude-plus/daemon.sock')
    expect(kinds(wiring)).toEqual(['session.start'])
  })

  test('/plus opens the pane on demand and remembers a main-screen terminal', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    const out = await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: false, columns: 120 } })
    expect(out.text).toBe('✻ claude-plus is open above the prompt')
    await start($)
    await clock.settle()
    expect(wiring.opened).toEqual(['plus'])
  })

  test('/plus on a wide fullscreen terminal says the pane is beside the conversation', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, {}, {})
    const out = await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
    await clock.settle()
    expect(out.text).toBe('✻ claude-plus is open beside the conversation')
  })

  test('the first session leaves the pane closed until the layout is known', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, {}, {})
    await start($)
    await clock.settle()
    expect(wiring.opened).toEqual([])
  })

  test('a pane opened unasked that lands on the main screen closes and stays closed', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 60, false)
    await clock.settle()
    expect([wiring.opened, wiring.closed]).toEqual([['plus'], ['plus']])
    await ui.unmount()
    wiring.session = 's2'
    await start($)
    await clock.settle()
    expect(wiring.opened).toEqual(['plus'])
  })

  test('a pane the person asked for on the main screen stays open', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, {}, {})
    await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: false, columns: 120 } })
    const ui = await mount($, 60, false)
    await clock.settle()
    expect(wiring.closed).toEqual([])
    await ui.unmount()
  })

  test('a reload neither opens the pane again nor forwards a second start for the session', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    await start($)
    await clock.advance(1000)
    expect(wiring.opened).toEqual(['plus'])
    expect(kinds(wiring)).toEqual(['session.start'])
  })

  test('/plus after a hot reload starts the ticker without a session start', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
    await clock.advance(1000)
    expect(wiring.ran).toEqual([['claude-plus', 'daemon', '--ensure']])
    expect(wiring.fetched.some(call => call.url.endsWith('/snapshot?session=s1'))).toBe(true)
    expect(wiring.statuses).toContain('$4.18 · cache 52:13 · 5h 54%')
  })

  test('events queued while the daemon is down are delivered once it answers', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { down: true })
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    expect(kinds(wiring)).toEqual([])
    wiring.down = false
    await clock.advance(1000)
    expect(kinds(wiring)).toEqual(['session.start', 'session.measure'])
  })

  test('a batch the daemon is slow to take is posted once, under its id', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.holding = true
    await $.session.measure(MEASURE)
    await clock.advance(500)
    await clock.advance(4000)
    const measures = () => posted(wiring.fetched).filter(event => event.kind === 'session.measure')
    expect(measures().length).toBe(1)
    wiring.holding = false
    for (const release of wiring.held.splice(0)) release()
    await clock.advance(2000)
    expect(measures().length).toBe(1)
    const ids = posted(wiring.fetched).map(event => event.id)
    expect(ids.every(id => typeof id === 'string' && id.length > 0)).toBe(true)
    expect(new Set(ids).size).toBe(ids.length)
  })

  test('a batch whose request failed after the timeout is sent again', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.holding = true
    await $.session.measure(MEASURE)
    await clock.advance(1500)
    wiring.holding = false
    wiring.down = true
    for (const release of wiring.held.splice(0)) release()
    await clock.settle()
    wiring.down = false
    await clock.advance(1000)
    expect(kinds(wiring)).toEqual(['session.start', 'session.measure'])
  })

  for (const columns of [40, 60, 100]) {
    test(`the pane draws the daemon snapshot at ${columns} columns`, async ($, on) => {
      const clock = mock.clock(on, { now: NOW_MS })
      wire(on)
      await start($)
      await $.session.measure(MEASURE)
      await clock.advance(1000)
      const ui = await mount($, columns)
      expect((await ui.find({ key: 'tab-live' }))?.props).toEqual({ key: 'tab-live', hotkey: '1', plain: true, dimColor: false, label: 'Live' })
      const rows = paintTree(await ui.drawn(), columns)
      expect(rows.filter(row => row.length > columns)).toEqual([])
      expect(trimmed(rows)).toEqual(PANE_FRAMES[columns])
      await ui.unmount()
    })
  }

  test('a measure sets the status line and is forwarded to the daemon', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    expect(wiring.statuses).toContain('$4.18 · cache 52:13 · 5h 54%')
    expect(kinds(wiring)).toEqual(['session.start', 'session.measure'])
    const measured = wiring.fetched.find(call => call.init?.body?.includes('session.measure'))
    expect(JSON.parse(measured?.init?.body ?? '{}').events[0].data).toEqual({
      model: 'claude-opus-5-5',
      context: { tokens: 46_000, window: 200_000, percent: 23 },
      rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAt: MEASURE.rateLimits[0]?.resetsAt }],
      cost_usd: 4.18,
      changed: ['context'],
    })
  })

  test('cost and the 5h percent keep moving while the daemon is down', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    expect(wiring.statuses.at(-1)).toBe('$4.18 · cache 52:13 · 5h 54%')
    wiring.down = true
    await clock.advance(3000)
    const later = { ...MEASURE.rateLimits[0]!, percentUsed: 71 }
    await $.session.measure({ ...MEASURE, rateLimits: [later], cost: { usd: 9.8 } })
    await clock.advance(1000)
    expect(wiring.statuses.at(-1)).toBe('$9.80 · cache 52:09 · 5h 71%')
  })

  test('the status countdown never skips a second while polls take time', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { lag: () => clock.sleep(300) })
    await start($)
    for (let step = 0; step < 200; step += 1) await clock.advance(100)
    const seconds = wiring.statuses.flatMap(status => {
      const found = /cache (\d+):(\d+)/.exec(status ?? '')
      return found === null ? [] : [Number(found[1]) * 60 + Number(found[2])]
    })
    const steps = seconds.slice(1).map((second, index) => (seconds[index] ?? 0) - second)
    expect(steps.filter(step => step !== 1)).toEqual([])
    expect(seconds.length).toBeGreaterThan(15)
  })

  test('a refused state read does not stop the ticker', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    let refusing = false
    on('state.get', ($, e, next) => (refusing ? { deny: 'state is away' } : next(e)))
    await start($)
    await clock.advance(1000)
    refusing = true
    await clock.advance(2000)
    refusing = false
    const before = wiring.statuses.length
    await clock.advance(5000)
    expect(wiring.statuses.length).toBeGreaterThan(before)
  })

  test('/plus answers even when the pane is refused', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { openRefusal: 'no panes in this session' })
    const out = await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
    await clock.settle()
    expect(out.text).toBe('✻ claude-plus could not open: no panes in this session')
  })

  test('the cache countdown ticks every second without a new snapshot', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'cache' }))?.text).toContain('52:13 left')
    await clock.advance(1000)
    expect((await ui.find({ key: 'cache' }))?.text).toContain('52:12 left')
  })

  test('a question mark opens and closes its explanation', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 60)
    expect(await ui.find({ key: 'explain-forecast' })).toBeUndefined()
    await ui.press({ key: 'why-forecast' })
    expect((await ui.find({ key: 'explain-forecast' }))?.text).toContain('typical of your last 10 requests')
    await ui.press({ key: 'why-forecast' })
    expect(await ui.find({ key: 'explain-forecast' })).toBeUndefined()
  })

  test('a turn step forwards its usage and restarts the local cache clock', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { served: null })
    on('turn.step', async function* () {
      return {
        turnId: 't1',
        index: 0,
        answer: '',
        toolUses: [],
        stopReason: 'end_turn',
        usage: { model: 'claude-opus-5-5', input_tokens: 12, output_tokens: 800, cache_read_input_tokens: 25_000, cache_creation_input_tokens: 300 },
      }
    })
    await start($)
    await $.session.measure(MEASURE)
    const stream = $.turn.step({ turnId: 't1', index: 0, model: 'claude-opus-5-5', effort: 'high', messageCount: 3 })
    let step = await stream.next()
    while (step.done !== true) step = await stream.next()
    await clock.advance(1000)
    expect(kinds(wiring)).toContain('turn.step')
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'cache' }))?.text).toBe('Cache ✻ warm (~1h)        59:59 left?')
    expect((await ui.find({ key: 'session-step' }))?.text).toBe('  Last request      25.3k in · 800 out')
  })

  test('the countdown counts from when the request was sent, not from when it ended', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { served: null })
    on('turn.step', async function* () {
      await clock.sleep(60_000)
      return {
        turnId: 't1',
        index: 0,
        answer: '',
        toolUses: [],
        stopReason: 'end_turn',
        usage: { model: 'claude-opus-5-5', input_tokens: 12, output_tokens: 800, cache_read_input_tokens: 25_000, cache_creation_input_tokens: 300 },
      }
    })
    await start($)
    await $.session.measure(MEASURE)
    const stream = $.turn.step({ turnId: 't1', index: 0, model: 'claude-opus-5-5', effort: 'high', messageCount: 3 })
    const drained = (async () => {
      let step = await stream.next()
      while (step.done !== true) step = await stream.next()
    })()
    await clock.advance(60_000)
    await drained
    await clock.advance(1000)
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'cache' }))?.text).toContain('58:59 left')
    const forwarded = wiring.delivered
      .flatMap(call => (JSON.parse(call.init?.body ?? '{}') as { events: { kind: string; ts_ms: number; data: { sent_ms?: number } }[] }).events)
      .find(event => event.kind === 'turn.step')
    expect([forwarded?.data.sent_ms, forwarded?.ts_ms]).toEqual([NOW_MS, NOW_MS + 60_000])
  })

  test('a missing binary shows how to install it and the pane still works', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { exitCode: 127, down: true })
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('claude-plus is not installed yet. Install it, or set its path in the plugin options')
    expect((await ui.find({ key: 'context' }))?.text).toBe('Context                     46k / 200k')
  })

  test('a daemon older than the pane is replaced and read again', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.serving = 'older'
    await clock.advance(3000)
    await clock.advance(1000)
    expect(wiring.ran).toHaveLength(2)
    const ui = await mount($, 40)
    expect(await ui.find({ key: 'daemon' })).toBeUndefined()
    expect((await ui.find({ key: 'headline' }))?.text).toContain('$4.18')
    await ui.unmount()
  })

  test('an old claude-plus binary gets one steady notice to update it and no endless restarts', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { serving: 'older', ensureCures: false })
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    const shown: (string | undefined)[] = []
    for (let second = 0; second < 120; second += 1) {
      await clock.advance(1000)
      shown.push((await ui.find({ key: 'daemon' }))?.text)
    }
    expect([...new Set(shown)]).toEqual(['The claude-plus daemon is older than this pane'])
    expect((await ui.find({ key: 'daemon-fix' }))?.text).toBe('Update the claude-plus binary')
    expect(wiring.ran).toHaveLength(2)
    await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
    await clock.advance(1000)
    expect(wiring.ran).toHaveLength(3)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('The claude-plus daemon is older than this pane')
    await ui.unmount()
  })

  test('a daemon newer than the pane asks for a plugin update and keeps running', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { serving: 'newer' })
    await start($)
    await clock.advance(60_000)
    expect(wiring.ran).toHaveLength(1)
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('The claude-plus daemon is newer than this pane')
    expect((await ui.find({ key: 'daemon-fix' }))?.text).toBe('Update the plus plugin')
    await ui.unmount()
  })

  test('a daemon that stops answering later is started again', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.down = true
    await clock.advance(3000)
    expect(wiring.ran).toHaveLength(1)
    await clock.advance(30_000)
    expect(wiring.ran).toHaveLength(2)
    await $.command.run({ command: 'plus', args: '', origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 160 } })
    await clock.settle()
    expect(wiring.ran).toHaveLength(3)
  })

  test('a connection the daemon closed while idle keeps the pane as it was', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.resets = 1
    await clock.advance(3000)
    const ui = await mount($, 40)
    expect(await ui.find({ key: 'daemon' })).toBeUndefined()
    expect((await ui.find({ key: 'headline' }))?.text).toContain('$4.18')
    await ui.unmount()
  })

  test('hooks return at once while the daemon is silent', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.silent = true
    let measured = false
    const measuring = $.session.measure(MEASURE).then(() => {
      measured = true
    })
    await clock.settle()
    expect(measured).toBe(true)
    await measuring
    let completed = false
    const completing = $.turn.complete({ turnId: 't1', reason: 'answer', answer: '', durationMs: 900, isAborted: false }).then(() => {
      completed = true
    })
    await clock.settle()
    expect(completed).toBe(true)
    await completing
  })

  test('a silent daemon is reported once the 500 ms timeout passes', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    wiring.silent = true
    await clock.advance(3000)
    await clock.advance(500)
    const ui = await mount($, 60)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('The daemon is not answering')
    expect((await ui.find({ key: 'daemon-fix' }))?.text).toBe('Run claude-plus daemon --ensure')
  })

  test('numbers ease to a new snapshot over 300 ms', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    const served = snapshot()
    wiring.served = { ...served, totals: { ...served.totals, cost_usd: 5.18 } }
    await clock.advance(2050)
    const midway = (await ui.find({ key: 'headline' }))?.text ?? ''
    expect(midway.endsWith('$5.18')).toBe(false)
    await clock.advance(300)
    expect((await ui.find({ key: 'headline' }))?.text.endsWith('$5.18')).toBe(true)
  })

  test('the heartbeat breathes while the cache is warm', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    const glyphs: string[] = []
    for (let second = 0; second < 4; second += 1) {
      glyphs.push(glyph(await ui.find({ key: 'cache' })))
      await clock.advance(1000)
    }
    expect(glyphs.map(drawn => JSON.parse(drawn).props)).toEqual([
      { color: 'claude' },
      { color: 'claude', bold: true },
      { color: 'claude' },
      { color: 'claude', dimColor: true },
    ])
  })

  test('animations=false jumps to new numbers and keeps the heartbeat still', { options: { animations: false } }, async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    const glyphs: unknown[] = []
    for (let second = 0; second < 4; second += 1) {
      glyphs.push(glyph(await ui.find({ key: 'cache' })))
      await clock.advance(1000)
    }
    expect(new Set(glyphs).size).toBe(1)
    const served = snapshot()
    wiring.served = { ...served, totals: { ...served.totals, cost_usd: 5.18 } }
    await clock.advance(3000)
    expect((await ui.find({ key: 'headline' }))?.text.endsWith('$5.18')).toBe(true)
  })

  test('a new session after /clear drops the previous snapshot', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    expect((await ui.find({ key: 'headline' }))?.text.startsWith('Opus 5.5 · high')).toBe(true)
    wiring.session = 's2'
    await clock.advance(3000)
    expect(await ui.find({ key: 'next' })).toBeUndefined()
    expect(wiring.fetched.some(call => call.url.endsWith('session=s2'))).toBe(true)
  })

  test('a rebuilding daemon keeps the last numbers and says so calmly', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 60)
    wiring.rebuilding = true
    await clock.advance(3000)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('Rebuilding the claude-plus index… numbers return in a moment')
    expect((await ui.find({ key: 'headline' }))?.text.endsWith('$4.18')).toBe(true)
    wiring.rebuilding = false
    await clock.advance(3000)
    expect(await ui.find({ key: 'daemon' })).toBeUndefined()
  })
})
