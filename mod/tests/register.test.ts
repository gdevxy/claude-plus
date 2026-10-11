import { describe, expect, mock, test } from 'claude-code/testing'
import type { Engine, FoundElement } from 'claude-code/testing'
import type { HttpInit, On, SessionUsageArgs } from 'claude-code'

import type { ContextReport, HistoryReport, Snapshot } from '../types'
import { historyScreen } from '../hooks/history'
import { MINUTE_MS, NOW_MS, contextReport, dayReport, historyReport, local, snapshot, usage } from './fixtures'
import { paintTree, painted, trimmed } from './paint'

type Wiring = {
  session: string
  served: Snapshot | null
  serving: 'current' | 'older' | 'newer'
  lag: (() => Promise<void>) | null
  historyLag: (() => Promise<void>) | null
  historyStatus: number | null
  openRefusal: string | null
  resets: number
  ensureCures: boolean
  report: ContextReport
  histories: Record<string, HistoryReport>
  theme: string
  usages: SessionUsageArgs[]
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
  placed: boolean
  duringUsage: (() => void) | null
  reportSession: string | null
}

const MEASURE = {
  context: { tokens: 46_000, window: 200_000, percent: 23 },
  rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAt: new Date(NOW_MS + 130 * MINUTE_MS).toISOString() }],
  cost: { usd: 4.18 },
  changed: ['context' as const],
}

const HISTORY_HEADERS: Record<number, string> = {
  40: ' ✻        1: Live 2: Context 3: History',
  60: ' ✻ claude-plus                1: Live 2: Context 3: History',
  100: ' ✻ claude-plus                                                        1: Live 2: Context 3: History',
}

const PANE_FRAMES: Record<number, string[]> = {
  40: [
    ' ✻        1: Live 2: Context 3: History',
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
    ' ✻ claude-plus                1: Live 2: Context 3: History',
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
    ' ✻ claude-plus                                                        1: Live 2: Context 3: History',
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

const CONTEXT_FRAMES: Record<number, string[]> = {
  40: [
    ' ✻        1: Live 2: Context 3: History',
    '',
    ' Context · Opus 5.5          46k / 200k',
    ' ███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    ' Auto-compact at 167k · 121k to go',
    '',
    ' By category                          ?',
    '   System prompt █░░░░░░░░░░░░░    3.1k',
    '   System tools  ████▍░░░░░░░░░   14.2k',
    ' ▸ MCP tools     █▎░░░░░░░░░░░░    4.1k',
    ' ▸ Memory files  ▋░░░░░░░░░░░░░      2k',
    ' ▸ Skills        ▊░░░░░░░░░░░░░    2.6k',
    '   Messages      ██████▏░░░░░░░     20k',
    '   Free space                      121k',
    '   Autocompact buffer               33k',
    ' + 8.4k of tool schemas load on demand',
    ' ~12.4k injected later · 1 compaction',
    '',
    ' Largest items                        ?',
    '   MCP linear · 2 tools            4.1k',
    '   Skill superpowers:brainstorming 2.3k',
    '   CLAUDE.md · Project             1.6k',
    '   Agent reviewer                   120',
    '',
    ' Not used in 14 days        2.3k a turn',
    '   Skill superpowers:brainstorming 2.3k',
    '',
    ' Largest tool results                 ?',
    '   Read                            ~12k',
    '   linear search                  ~4.2k',
    '',
    ' Configured                           ?',
    '   CLAUDE.md files      3 · 1 on demand',
    '   Skills              66 · 2 on demand',
    '   MCP servers                        3',
    '   Plugins                    5 · 1 off',
  ],
  60: [
    ' ✻ claude-plus                1: Live 2: Context 3: History',
    '',
    ' Context · Opus 5.5                              46k / 200k',
    ' ████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    ' Auto-compact at 167k · 121k to go',
    '',
    ' By category                                              ?',
    '   System prompt ██▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    3.1k',
    '   System tools  ██████████▌░░░░░░░░░░░░░░░░░░░░░░░   14.2k',
    ' ▸ MCP tools     ███░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    4.1k',
    ' ▸ Memory files  █▌░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░      2k',
    ' ▸ Skills        █▉░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    2.6k',
    '   Messages      ██████████████▊░░░░░░░░░░░░░░░░░░░     20k',
    '   Free space                                          121k',
    '   Autocompact buffer                                   33k',
    ' + 8.4k of tool schemas load on demand',
    ' ~12.4k injected since your first request · 1 compaction',
    '',
    ' Largest items                                            ?',
    '   MCP linear · 2 tools                                4.1k',
    '   Skill superpowers:brainstorming                     2.3k',
    '   CLAUDE.md · Project                                 1.6k',
    '   Agent reviewer                                       120',
    '',
    ' Not used in 14 days                            2.3k a turn',
    '   Skill superpowers:brainstorming                     2.3k',
    '',
    ' Largest tool results                                     ?',
    '   Read                                                ~12k',
    '   linear search                                      ~4.2k',
    '',
    ' Configured                                               ?',
    '   CLAUDE.md files                          3 · 1 on demand',
    '   Skills                                  66 · 2 on demand',
    '   MCP servers                                            3',
    '   Plugins                                        5 · 1 off',
  ],
  100: [
    ' ✻ claude-plus                                                        1: Live 2: Context 3: History',
    '',
    ' Context · Opus 5.5                                                                      46k / 200k',
    ' █████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
    ' Auto-compact at 167k · 121k to go',
    '',
    ' By category                                                                                      ?',
    '   System prompt █████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    3.1k',
    '   System tools  ██████████████████████▉░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   14.2k',
    ' ▸ MCP tools     ██████▋░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    4.1k',
    ' ▸ Memory files  ███▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░      2k',
    ' ▸ Skills        ████▏░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░    2.6k',
    '   Messages      ████████████████████████████████▏░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░     20k',
    '   Free space                                                                                  121k',
    '   Autocompact buffer                                                                           33k',
    ' + 8.4k of tool schemas load on demand',
    ' ~12.4k injected since your first request · 1 compaction',
    '',
    ' Largest items                                                                                    ?',
    '   MCP linear · 2 tools                                                                        4.1k',
    '   Skill superpowers:brainstorming                                                             2.3k',
    '   CLAUDE.md · Project                                                                         1.6k',
    '   Agent reviewer                                                                               120',
    '',
    ' Not used in 14 days                                                                    2.3k a turn',
    '   Skill superpowers:brainstorming                                                             2.3k',
    '',
    ' Largest tool results                                                                             ?',
    '   Read                                                                                        ~12k',
    '   linear search                                                                              ~4.2k',
    '',
    ' Configured                                                                                       ?',
    '   CLAUDE.md files                                                                  3 · 1 on demand',
    '   Skills                                                                          66 · 2 on demand',
    '   MCP servers                                                                                    3',
    '   Plugins                                                                                5 · 1 off',
  ],
}

function wire(on: On, overrides: Partial<Wiring> = {}, stored: Record<string, unknown> = { fullscreen: true }): Wiring {
  const wiring: Wiring = { session: 's1', served: snapshot(), report: contextReport(), histories: { '7d': historyReport(), '24h': dayReport() }, theme: 'dark', usages: [], serving: 'current', lag: null, historyLag: null, historyStatus: null, openRefusal: null, resets: 0, ensureCures: true, silent: false, holding: false, held: [], down: false, rebuilding: false, exitCode: 0, fetched: [], delivered: [], ran: [], opened: [], closed: [], statuses: [], placed: true, duringUsage: null, reportSession: null, ...overrides }
  mock.env(on, { HOME: '/Users/me' })
  mock.store(on, stored)
  on('session.id', () => ({ value: wiring.session }))
  on('config.list', () => ({ value: [{ key: 'theme', label: 'Theme', kind: 'choice', value: wiring.theme, provider: { plugin: 'engine', tier: 'core' }, isLocked: false }] }))
  on('config.set', ($, e) => ({ value: e.value }))
  on('session.model', () => ({ value: 'claude-opus-5-5' }))
  on('session.turns', () => ({ value: 14 }))
  on('session.usage', ($, e) => {
    wiring.usages.push(e)
    wiring.duringUsage?.()
    return { value: usage() }
  })
  on('command.register', ($, e) => ({ value: { command: e.name } }))
  on('ui.open', ($, e) => {
    if (wiring.openRefusal !== null) return { deny: wiring.openRefusal }
    wiring.opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.panes', () => ({ value: wiring.placed ? [{ id: 'plus', title: 'claude-plus', isShown: true, isFocused: false, isPlaced: true }] : [] }))
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
    if (wiring.historyLag !== null && e.url.includes('/history')) await wiring.historyLag()
    if (wiring.holding && e.url.endsWith('/events')) await new Promise<void>(resolve => wiring.held.push(resolve))
    if (wiring.down) throw new Error('ECONNREFUSED')
    if (wiring.resets > 0) {
      wiring.resets -= 1
      throw new Error('ECONNRESET')
    }
    if (e.url.endsWith('/events')) wiring.delivered.push(e)
    if (wiring.rebuilding && e.url.includes('/snapshot')) return { value: { status: 503, ok: false, headers: {}, text: '{"error":"rebuilding"}' } }
    if (wiring.historyStatus !== null && e.url.includes('/history')) return { value: { status: wiring.historyStatus, ok: false, headers: {}, text: '{"error":"no route for GET /history"}' } }
    const body = e.url.includes('/snapshot')
      ? JSON.stringify(servedBody(wiring))
      : e.url.includes('/history')
        ? JSON.stringify({ v: 1, ...historyOf(wiring, e.url) })
        : e.url.includes('/context')
        ? JSON.stringify({ v: 1, ...wiring.report, session: wiring.reportSession ?? wiring.session })
        : '{"accepted":1,"rejected":[]}'
    return { value: { status: 200, ok: true, headers: {}, text: body } }
  })
  on('ui.render', { component: 'PromptHint' }, () => ({ type: 'Text', props: {}, children: ['? for shortcuts'] }))
  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('session.measure', ($, e) => ({ changed: e.changed }))
  on('turn.start', ($, e) => ({ turnId: e.turnId }))
  on('turn.complete', () => ({ text: '' }))
  return wiring
}

function historyOf(wiring: Wiring, url: string): HistoryReport {
  const range = new URL(url).searchParams.get('range') ?? '7d'
  return wiring.histories[range] ?? { ...historyReport({ range: range as HistoryReport['range'] }), buckets: [], totals: { ...historyReport().totals, requests: 0, cost_usd: 0, reconciled_usd: 0 } }
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

async function hint($: Engine, fullscreen: boolean): Promise<void> {
  const drawn = await $.ui.mount({
    plugin: 'plus',
    surface: 'terminal',
    component: 'PromptHint',
    requestId: 'hint',
    props: { isDraft: false, isWorking: false, hint: '? for shortcuts' },
    viewport: { columns: 160, rows: 50, isFullscreen: fullscreen },
  })
  await drawn.unmount()
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

type Posted = { id?: string; kind: string; session?: string }

function posted(calls: { url: string; init?: HttpInit }[]): Posted[] {
  return calls
    .filter(call => call.url.endsWith('/events'))
    .flatMap(call => (JSON.parse(call.init?.body ?? '{}') as { events: Posted[] }).events)
}

function kinds(wiring: Wiring): string[] {
  return posted(wiring.delivered).map(event => event.kind)
}

describe('plus mod', () => {
  test('a session start ensures the daemon, registers /plus and opens the pane once the layout is fullscreen', { options: { binaryPath: '/opt/bin/claude-plus' } }, async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, {}, {})
    await start($)
    await clock.settle()
    expect(wiring.ran).toEqual([['/opt/bin/claude-plus', 'daemon', '--ensure']])
    expect(wiring.opened).toEqual([])
    await hint($, true)
    await clock.settle()
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

  test('a main-screen session never opens the pane unasked, even after a fullscreen one', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await hint($, false)
    await clock.advance(1000)
    await hint($, true)
    await clock.settle()
    expect(wiring.opened).toEqual([])
  })

  test('a pane opened unasked that a surface still seats inline closes and stays closed', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await hint($, true)
    await clock.advance(1000)
    const ui = await mount($, 60, false)
    await clock.settle()
    expect([wiring.opened, wiring.closed]).toEqual([['plus'], ['plus']])
    await ui.unmount()
    wiring.session = 's2'
    await start($)
    await hint($, true)
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
    await hint($, true)
    await clock.advance(1000)
    await start($)
    await hint($, true)
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
      expect((await ui.find({ key: 'tab-context' }))?.props).toEqual({ key: 'tab-context', hotkey: '2', plain: true, dimColor: true, label: 'Context' })
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

  for (const columns of [40, 60, 100]) {
    test(`the Context tab opens the view and draws the breakdown at ${columns} columns`, async ($, on) => {
      const clock = mock.clock(on, { now: NOW_MS })
      const wiring = wire(on)
      await start($)
      await $.session.measure(MEASURE)
      await clock.advance(1000)
      const ui = await mount($, columns)
      await ui.press({ key: 'tab-context' })
      await clock.advance(1000)
      expect(wiring.usages).toEqual([{ breakdown: 'full', columns: 40 }])
      expect((await ui.find({ key: 'tab-context' }))?.props.dimColor).toBe(false)
      expect((await ui.find({ key: 'ctx-head' }))?.text).toBe(`Context · Opus 5.5${' '.repeat(columns - 2 - 28)}46k / 200k`)
      expect((await ui.find({ key: 'ctx-cat-6' }))?.text.length).toBe(columns - 2)
      expect((await ui.find({ key: 'ctx-cat-4' }))?.text.length).toBe(columns - 2)
      expect((await ui.find({ key: 'ctx-top-0' }))?.text.startsWith('  MCP linear · 2 tools')).toBe(true)
      expect(wiring.fetched.some(call => call.url.endsWith('/context?session=s1'))).toBe(true)
      expect(paintTree(await ui.drawn(), columns).filter(row => row.length > columns)).toEqual([])
      await ui.press({ key: 'open-memory_file' })
      expect((await ui.find({ key: 'ctx-cat-4' }))?.text.startsWith('▾ Memory files')).toBe(true)
      expect((await ui.find({ key: 'ctx-cat-4-item-0' }))?.text).toBe(`    CLAUDE.md · Project${' '.repeat(columns - 2 - 27)}1.6k`)
      await ui.press({ key: 'open-memory_file' })
      expect(await ui.find({ key: 'ctx-cat-4-item-0' })).toBeUndefined()
      await ui.press({ key: 'tab-live' })
      expect(await ui.find({ key: 'ctx-head' })).toBeUndefined()
      expect((await ui.find({ key: 'headline' }))?.text.startsWith('Opus 5.5 · high')).toBe(true)
      await ui.unmount()
    })
  }

  for (const columns of [40, 60, 100]) {
    test(`the Context view draws its whole frame at ${columns} columns`, async ($, on) => {
      const clock = mock.clock(on, { now: NOW_MS })
      wire(on)
      await start($)
      await $.session.measure(MEASURE)
      await clock.advance(1000)
      const ui = await mount($, columns)
      await ui.press({ key: 'tab-context' })
      await clock.advance(1000)
      const rows = paintTree(await ui.drawn(), columns)
      expect(rows.filter(row => row.length > columns)).toEqual([])
      expect(trimmed(rows)).toEqual(CONTEXT_FRAMES[columns])
      await ui.unmount()
    })
  }

  test('the breakdown is forwarded as a session.usage event without its grid', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await $.turn.complete({ turnId: 't1', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(1000)
    await clock.advance(1000)
    const sent = wiring.delivered
      .flatMap(call => (JSON.parse(call.init?.body ?? '{}') as { events: { kind: string; data: Record<string, unknown> }[] }).events)
      .find(event => event.kind === 'session.usage')
    expect(sent?.data.gridRows).toBeUndefined()
    expect(sent?.data.totalTokens).toBe(46_000)
    expect(sent?.data.contextWindow).toBe(200_000)
    expect((sent?.data.mcpTools as unknown[]).length).toBe(1)
  })

  test('the Live view asks for a breakdown once per ten minutes of turns, the Context view once a minute', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    expect(wiring.usages.length).toBe(0)
    await $.turn.complete({ turnId: 't1', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(1000)
    expect(wiring.usages.length).toBe(1)
    await $.turn.complete({ turnId: 't2', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(5 * MINUTE_MS)
    expect(wiring.usages.length).toBe(1)
    await clock.advance(5 * MINUTE_MS)
    expect(wiring.usages.length).toBe(2)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-context' })
    await $.turn.complete({ turnId: 't3', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(30_000)
    expect(wiring.usages.length).toBe(2)
    await clock.advance(30_000)
    expect(wiring.usages.length).toBe(3)
    await $.turn.complete({ turnId: 'a1', agentId: 'agent-1', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(2 * MINUTE_MS)
    expect(wiring.usages.length).toBe(3)
  })

  test('a pane that is not shown asks for no breakdown and polls no context report', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-context' })
    await clock.advance(1000)
    expect(wiring.usages.length).toBe(1)
    await ui.unmount()
    wiring.placed = false
    const polled = () => wiring.fetched.filter(call => call.url.includes('/context')).length
    const before = polled()
    for (const turnId of ['t1', 't2', 't3']) {
      await $.turn.complete({ turnId, reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
      await clock.advance(5 * MINUTE_MS)
    }
    expect(wiring.usages.length).toBe(1)
    expect(polled()).toBe(before)
    wiring.placed = true
    await clock.advance(1000)
    expect(wiring.usages.length).toBe(2)
    expect(polled()).toBeGreaterThan(before)
  })

  test('the Live view stops its background breakdowns while the pane is not shown', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { placed: false })
    await start($)
    await $.turn.complete({ turnId: 't1', reason: 'answer', answer: '', durationMs: 1000, isAborted: false })
    await clock.advance(11 * MINUTE_MS)
    expect(wiring.usages.length).toBe(0)
    wiring.placed = true
    await clock.advance(1000)
    expect(wiring.usages.length).toBe(1)
  })

  test('a breakdown that finishes after /clear is dropped instead of filed under the new session', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { report: contextReport({ live: null }) })
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-context' })
    wiring.duringUsage = () => {
      wiring.session = 's2'
    }
    await clock.advance(1000)
    await clock.advance(3000)
    expect(wiring.usages.length).toBe(1)
    expect(posted(wiring.delivered).filter(event => event.kind === 'session.usage')).toEqual([])
    expect(await ui.find({ key: 'ctx-cat-6' })).toBeUndefined()
  })

  test('the Context view keeps the breakdown from Claude Code while the daemon is down', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { down: true })
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-context' })
    await clock.advance(1000)
    await clock.advance(3000)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('The daemon is not answering')
    expect((await ui.find({ key: 'daemon-fix' }))?.text).toBe('Run claude-plus daemon --ensure')
    expect((await ui.find({ key: 'ctx-head' }))?.text).toBe('Context · Opus 5.5          46k / 200k')
    expect(await ui.find({ key: 'ctx-top' })).toBeUndefined()
  })

  test('a new session after /clear drops the previous context report', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-context' })
    await clock.advance(1000)
    expect(await ui.find({ key: 'ctx-top' })).toBeDefined()
    wiring.session = 's2'
    wiring.down = true
    await clock.advance(3000)
    expect(await ui.find({ key: 'ctx-top' })).toBeUndefined()
    expect(await ui.find({ key: 'ctx-cat-6' })).toBeUndefined()
  })

  test('a late context report of the session before /clear is ignored', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-context' })
    await clock.advance(1000)
    expect(await ui.find({ key: 'ctx-top' })).toBeDefined()
    wiring.session = 's2'
    wiring.reportSession = 's1'
    await clock.advance(3000)
    await clock.advance(10_000)
    expect(wiring.fetched.some(call => call.url.endsWith('/context?session=s2'))).toBe(true)
    expect(await ui.find({ key: 'ctx-top' })).toBeUndefined()
    expect(await ui.find({ key: 'ctx-cat-6' })).toBeUndefined()
    expect((await ui.find({ key: 'ctx-head' }))?.text).toBe('Context')
  })
  for (const columns of [40, 60, 100]) {
    test(`the History tab draws raster charts at ${columns} columns`, async ($, on) => {
      const clock = mock.clock(on, { now: NOW_MS })
      const wiring = wire(on)
      await start($)
      await $.session.measure(MEASURE)
      await clock.advance(1000)
      const ui = await mount($, columns)
      await ui.press({ key: 'tab-history' })
      await clock.advance(1000)
      expect((await ui.find({ key: 'tab-history' }))?.props).toEqual({ key: 'tab-history', hotkey: '3', plain: true, dimColor: false, label: 'History' })
      expect((await ui.find({ key: 'range' }))?.props).toEqual({ key: 'range', hotkey: 'r', plain: true, label: '7 days' })
      expect((await ui.find({ key: 'metric' }))?.props).toEqual({ key: 'metric', hotkey: 'g', plain: true, label: 'Cost' })
      expect(wiring.fetched.filter(call => call.url.includes('/history')).map(call => call.url)).toEqual(['http://claude-plus/history?range=7d'])
      for (const key of ['hist-spend-chart', 'hist-hit-chart', 'hist-peak-chart', 'hist-growth-chart']) {
        const chart = await ui.find({ key })
        expect([key, chart?.type, chart?.props.columns]).toEqual([key, 'Raster', columns - 2])
      }
      const rows = paintTree(await ui.drawn(), columns)
      expect(rows.filter(row => row.length > columns)).toEqual([])
      expect(trimmed(rows.slice(0, 4))).toEqual([HISTORY_HEADERS[columns], '', ' r: 7 days  g: Cost', ''])
      const screen = historyScreen({ width: columns, range: '7d', metric: 'cost', reports: { '7d': historyReport() }, failures: {}, context: { ...contextReport(), session: 's1' }, local: local(), daemon: { status: 'ok', detail: null }, expanded: [] })
      expect(trimmed(rows.slice(4))).toEqual(trimmed(painted(screen, columns)))
      await ui.unmount()
    })
  }

  test('the History tab draws the last 24 hours mounted as the screen lays them out', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-history' })
    for (const _ of [0, 1, 2]) await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'range' }))?.props.label).toBe('24 hours')
    expect((await ui.find({ key: 'hist-hit-chart' }))?.type).toBe('Raster')
    const rows = paintTree(await ui.drawn(), 60)
    const screen = historyScreen({ width: 60, range: '24h', metric: 'cost', reports: { '24h': dayReport() }, failures: {}, context: { ...contextReport(), session: 's1' }, local: local(), daemon: { status: 'ok', detail: null }, expanded: [] })
    expect(trimmed(rows.slice(4))).toEqual(trimmed(painted(screen, 60)))
    expect(rows.some(row => row.includes('▇'))).toBe(true)
    await ui.unmount()
  })

  test('a range chosen while a report is on its way is fetched as soon as that one lands', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { historyLag: () => clock.sleep(2000) })
    wiring.histories['30d'] = historyReport({ range: '30d' })
    const asked = () => wiring.fetched.filter(call => call.url.includes('/history')).map(call => call.url.split('=')[1])
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    await clock.advance(500)
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect(asked()).toEqual(['7d'])
    for (let step = 0; step < 40; step += 1) await clock.advance(100)
    expect(asked()).toEqual(['7d', '30d'])
    expect((await ui.find({ key: 'hist-head' }))?.text.startsWith('Last 30 days')).toBe(true)
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
  })

  test('a hidden pane asks for no History report until it is shown again', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    const asked = () => wiring.fetched.filter(call => call.url.includes('/history')).length
    await start($)
    const ui = await mount($, 40)
    wiring.placed = false
    await ui.press({ key: 'tab-history' })
    await clock.advance(90_000)
    expect(asked()).toBe(0)
    wiring.placed = true
    await clock.advance(1000)
    expect(asked()).toBe(1)
  })

  test('r cycles the range and fetches it at once, g cycles what the bars count', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    wiring.histories['30d'] = historyReport()
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    await ui.press({ key: 'metric' })
    expect((await ui.find({ key: 'metric' }))?.props.label).toBe('Tokens')
    expect((await ui.find({ key: 'hist-head' }))?.text.endsWith('1.6M')).toBe(true)
    await ui.press({ key: 'metric' })
    expect((await ui.find({ key: 'metric' }))?.props.label).toBe('% of 5h')
    await ui.press({ key: 'metric' })
    expect((await ui.find({ key: 'metric' }))?.props.label).toBe('Cost')
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'range' }))?.props.label).toBe('30 days')
    expect(wiring.fetched.filter(call => call.url.includes('/history')).map(call => call.url.split('=')[1])).toEqual(['7d', '30d'])
    expect((await ui.find({ key: 'hist-loading' }))?.text).toBe('Loading the last 30 days… ✻')
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'hist-empty' }))?.text).toBe('No requests in the last 90 days yet ✻')
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'hist-spend' }))?.text.startsWith('Cost by hour')).toBe(true)
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'hist-head' }))?.text.startsWith('Last 7 days')).toBe(true)
  })

  test('g skips the 5h share until the limit is calibrated', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { histories: { '7d': historyReport({ calibration: { calibrated: false, pairs: 4, needed: 20 } }) } })
    await start($)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    await ui.press({ key: 'metric' })
    await ui.press({ key: 'metric' })
    expect((await ui.find({ key: 'metric' }))?.props.label).toBe('Cost')
  })

  test('the History view asks for its report every 30 s while shown and not at all from Live', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    const asked = () => wiring.fetched.filter(call => call.url.includes('/history')).length
    await start($)
    await clock.advance(60_000)
    expect(asked()).toBe(0)
    const ui = await mount($, 60)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    expect(asked()).toBe(1)
    await clock.advance(28_000)
    expect(asked()).toBe(1)
    await clock.advance(2000)
    expect(asked()).toBe(2)
    expect(wiring.fetched.some(call => call.url.includes('/context?session=s1'))).toBe(true)
    await ui.press({ key: 'tab-live' })
    await clock.advance(60_000)
    expect(asked()).toBe(2)
  })

  test('charts are text in theme colors where the theme has no known palette, and follow a theme change', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { theme: 'dark-ansi' })
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    const text = await ui.find({ key: 'hist-spend-chart' })
    expect(text?.type).toBe('Box')
    expect(paintTree(await ui.drawn(), 40).some(row => row.includes('▅▅▅▅ ▃▃▃▃'))).toBe(true)
    await $.config.set({ key: 'theme', value: 'light', previous: 'dark-ansi', provider: { plugin: 'engine', tier: 'core' }, origin: { kind: 'composer' } })
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
    await ui.unmount()
  })

  test('a pane wider than a Raster can be draws its charts as text instead of refusing the tree', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await $.session.measure(MEASURE)
    await clock.advance(1000)
    const ui = await mount($, 600)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    for (const key of ['hist-spend-chart', 'hist-hit-chart', 'hist-peak-chart', 'hist-growth-chart']) expect([key, (await ui.find({ key }))?.type]).toEqual([key, 'Box'])
    await ui.unmount()
    const fits = await mount($, 514)
    expect((await fits.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
    await fits.unmount()
  })

  test('the desktop draws the charts as text', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on)
    await start($)
    await clock.advance(1000)
    const ui = await $.ui.mount({
      plugin: 'plus',
      surface: 'desktop',
      component: 'Pane',
      requestId: 'plus',
      props: { title: 'claude-plus', isFocused: true, bodyColumns: 60, placement: 'dock', scroll: { offset: 0, bodyRows: 40 }, view: {} },
      viewport: { columns: 160, rows: 50, isFullscreen: true },
    })
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Box')
  })

  test('a History report that takes two seconds still arrives and the countdown never waits for it', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { historyLag: () => clock.sleep(2000) })
    await start($)
    await clock.advance(1000)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    for (let step = 0; step < 40; step += 1) await clock.advance(100)
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
    const seconds = wiring.statuses.flatMap(status => {
      const found = /cache (\d+):(\d+)/.exec(status ?? '')
      return found === null ? [] : [Number(found[1]) * 60 + Number(found[2])]
    })
    const steps = seconds.slice(1).map((second, index) => (seconds[index] ?? 0) - second)
    expect(steps.filter(step => step !== 1)).toEqual([])
  })

  test('a History report that comes back for another range is not shown', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    wiring.histories['30d'] = historyReport()
    wiring.histories['90d'] = historyReport()
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    await ui.press({ key: 'range' })
    await clock.advance(1000)
    expect((await ui.find({ key: 'hist-loading' }))?.text).toBe('Loading the last 30 days… ✻')
    expect(await ui.find({ key: 'hist-spend-chart' })).toBeUndefined()
  })

  test('a daemon without the History route is named and asked again every 30 s until it has one', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on, { historyStatus: 404 })
    const asked = () => wiring.fetched.filter(call => call.url.includes('/history')).length
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    expect(await ui.find({ key: 'hist-loading' })).toBeUndefined()
    expect((await ui.find({ key: 'hist-failed' }))?.text).toBe('This claude-plus daemon is older than the pane and has no History')
    expect((await ui.find({ key: 'hist-retry' }))?.text).toBe('Run claude-plus daemon stop to restart it')
    expect(asked()).toBe(1)
    await clock.advance(30_000)
    expect(asked()).toBe(2)
    wiring.historyStatus = null
    await clock.advance(30_000)
    expect(asked()).toBe(3)
    expect(await ui.find({ key: 'hist-failed' })).toBeUndefined()
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
  })

  test('a History report that takes longer than 3 s is cut off and reads as not loaded', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    wire(on, { historyLag: () => clock.sleep(5000) })
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    for (let step = 0; step < 29; step += 1) await clock.advance(100)
    expect((await ui.find({ key: 'hist-loading' }))?.text).toBe('Loading the last 7 days… ✻')
    for (let step = 0; step < 3; step += 1) await clock.advance(100)
    expect((await ui.find({ key: 'hist-failed' }))?.text).toBe('Could not load the last 7 days: no answer within 3000 ms')
    expect(await ui.find({ key: 'hist-spend-chart' })).toBeUndefined()
  })

  test('the History view keeps its last report while the daemon is down', async ($, on) => {
    const clock = mock.clock(on, { now: NOW_MS })
    const wiring = wire(on)
    await start($)
    const ui = await mount($, 40)
    await ui.press({ key: 'tab-history' })
    await clock.advance(1000)
    wiring.down = true
    await clock.advance(31_000)
    expect((await ui.find({ key: 'daemon' }))?.text).toBe('The daemon is not answering')
    expect((await ui.find({ key: 'hist-spend-chart' }))?.type).toBe('Raster')
  })
})
