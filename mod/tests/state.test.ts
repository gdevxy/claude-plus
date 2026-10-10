import { describe, expect, test } from 'claude-code/testing'

import type { Line, ScreenInput } from '../hooks/state'
import {
  EASE_MS,
  FRAME_MS,
  QUICK_BREATH_MS,
  TICK_MS,
  WHY_COLUMNS,
  cacheClock,
  eased,
  ensureError,
  ensureOutcome,
  heartbeat,
  retarget,
  screen,
  settled,
  statusText,
  tickPeriod,
  toggled,
} from '../hooks/state'
import { MINUTE_MS, NOW_MS, local, snapshot } from './fixtures'

const OK = { status: 'ok', detail: null } as const

function input(overrides: Partial<ScreenInput> = {}): ScreenInput {
  return { width: 40, nowMs: NOW_MS, animations: true, snapshot: snapshot(), local: local(), daemon: OK, expanded: [], motions: {}, ...overrides }
}

function text(line: Line): string {
  return line.segments.map(segment => segment.text).join('')
}

function frame(lines: Line[]): string[] {
  return lines.map(line => `${text(line)}${line.why === undefined ? '' : ' ?'}`)
}

function line(lines: Line[], key: string): Line {
  const found = lines.find(candidate => candidate.key === key)
  if (found === undefined) throw new Error(`no line ${key}`)
  return found
}

describe('live screen', () => {
  test('draws the whole session at 40 columns', () => {
    expect(frame(screen(input()))).toEqual([
      'Opus 5.5 · high                  $4.18',
      ' ',
      'Context                     46k / 200k',
      '███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
      ' ',
      'Cache ✻ warm · 1h         52:14 left ?',
      '█████████████████████████████████▏░░░░',
      'Cache hits saved you $3.20 this session',
      '  Hit ratio                        93%',
      '  Rebuilds                1 · ~$0.40 ?',
      ' ',
      'Next turn                            ?',
      '  if warm         ~$0.03 · <0.1% of 5h',
      '  if cold          ~$0.40 · 0.2% of 5h',
      '  on Sonnet 5.5    ~$0.20, then ~$0.02',
      ' ',
      'Session                               ',
      '  Tokens in / out             2M / 61k',
      '  Turns · subagents             14 · 2',
      ' ',
      'Plan limits                          ?',
      '  5h    █████████████▌░░░░░░░░░░░  54%',
      'resets in 2h 10m · on pace for 81%',
      '  7d    ███████▊░░░░░░░░░░░░░░░░░  31%',
      'resets in 3d',
    ])
  })

  for (const width of [40, 60, 100]) {
    test(`every unwrapped line fills exactly ${width - 2} columns`, () => {
      const lines = screen(input({ width, expanded: ['cache', 'rebuilds', 'forecast', 'limits'] }))
      for (const drawn of lines.filter(candidate => candidate.wrap !== true && !candidate.key.startsWith('gap-'))) {
        expect([drawn.key, text(drawn).length + (drawn.why === undefined ? 0 : WHY_COLUMNS)]).toEqual([drawn.key, width - 2])
      }
    })
  }

  test('never draws narrower than its minimum content width', () => {
    const lines = screen(input({ width: 12 }))
    expect(text(line(lines, 'headline'))).toBe('Opus 5.5 · high    $4.18')
  })

  test('styles the cost, the context bar and the countdown with theme keys', () => {
    const lines = screen(input({ local: local({ measure: { ...local().measure!, contextPercent: 85 } }) }))
    expect(line(lines, 'headline').segments[1]).toEqual({ text: '$4.18', color: 'claude', bold: true })
    expect(line(lines, 'context-bar').segments[1]?.color).toBe('warning')
    expect(line(lines, 'cache').segments[1]).toEqual({ text: '✻', color: 'claude', dim: true })
    expect(line(lines, 'cache').segments[3]).toEqual({ text: '52:14 left', color: 'success' })
    expect(line(lines, 'cache-bar').segments[0]?.color).toBe('success')
  })

  test('turns the countdown amber in its last minute and cold after it', () => {
    const urgent = screen(input({ nowMs: NOW_MS + 51 * MINUTE_MS + 30_000 }))
    expect(line(urgent, 'cache').segments[3]).toEqual({ text: '0:44 left', color: 'warning' })
    const cold = screen(input({ nowMs: NOW_MS + 53 * MINUTE_MS }))
    expect(frame(cold).slice(5, 8)).toEqual([
      'Cache · cold · 1h            expired ?',
      '░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░',
      'Cache went cold while you were away, rewarming will cost ~$0.40',
    ])
    expect(line(cold, 'next-warm').segments[1]?.dim).toBe(true)
    expect(line(cold, 'next-cold').segments[1]?.dim).toBeUndefined()
  })

  test('explains each number once its question mark is pressed', () => {
    const lines = screen(input({ expanded: ['cache', 'rebuilds', 'forecast', 'limits'] }))
    expect(text(line(lines, 'explain-cache'))).toBe(
      'The last cache write in the transcript asked for a 1 hour lifetime. Every request that reads or writes the cache starts the countdown again.',
    )
    expect(text(line(lines, 'explain-rebuilds'))).toBe(
      'The last rebuild, 40m ago, wrote the prompt again because the cache expired during an idle gap.',
    )
    expect(text(line(lines, 'explain-forecast'))).toBe(
      'Your 46k token prompt is read from the cache when warm and written again when cold. 2k new and 600 output tokens are the medians of your last 10 requests.',
    )
    expect(text(line(lines, 'explain-limits'))).toBe(
      'From 23 readings, 1% of your 5h window is about $2.00 of API-priced usage on Opus. The pace projects the last 30 minutes to the reset.',
    )
  })

  test('warns when a window will fill before it resets', () => {
    const [five, seven] = snapshot().live.limits
    const filling = { ...five!, used_pct: 82, pace: { pct_per_hour: 20, projected_pct: 125, full_at_ms: NOW_MS + 54 * MINUTE_MS } }
    const lines = screen(input({ snapshot: snapshot({ limits: [filling, seven!] }) }))
    expect(line(lines, 'limit-five_hour').segments[1]?.color).toBe('warning')
    expect(line(lines, 'limit-five_hour-detail')).toEqual({
      key: 'limit-five_hour-detail',
      segments: [{ text: 'resets in 2h 10m · full in 54m at this pace', color: 'warning' }],
      wrap: true,
      indent: 8,
    })
  })

  test('says it is still learning until the plan has enough readings', () => {
    const [five] = snapshot().live.limits
    const learning = { ...five!, calibration: { family: 'opus', pairs: 3, needed: 20, pct_per_usd: null } }
    const lines = screen(input({ snapshot: snapshot({ limits: [learning] }), expanded: ['limits'] }))
    expect(text(line(lines, 'limits-learning'))).toBe('Learning how your plan counts tokens… a few more turns ✻')
    expect(text(line(lines, 'explain-limits'))).toBe(
      'Each turn pairs the percent Claude Code reports with what it would cost on the API; 3 of 20 readings so far. The pace projects the last 30 minutes to the reset.',
    )
  })

  test('keeps working from Claude Code alone while the daemon is missing', () => {
    const step = { atMs: NOW_MS - 1000, model: 'claude-opus-5-5', input: 12, output: 800, cacheRead: 25_000, cacheWrite: 300 }
    const lines = screen(input({ snapshot: null, local: local({ step }), daemon: { status: 'missing', detail: null } }))
    expect(frame(lines)).toEqual([
      'claude-plus is not installed yet. Install it, or set its path in the plugin options',
      ' ',
      'Opus 5.5                         $4.18',
      ' ',
      'Context                     46k / 200k',
      '███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
      ' ',
      'Cache ✻ warm · ~1h        59:59 left ?',
      '██████████████████████████████████████',
      ' ',
      'Session                               ',
      '  Turns                             14',
      '  Last request      25.3k in · 800 out',
      ' ',
      'Plan limits                          ?',
      '  5h    █████████████▌░░░░░░░░░░░  54%',
      'resets in 2h 10m',
    ])
  })

  test('keeps cost, limits and effort fresh from Claude Code while the daemon is down', () => {
    const measure = {
      ...local().measure!,
      atMs: NOW_MS,
      costUsd: 9.8,
      rateLimits: [{ kind: 'five_hour', percentUsed: 71, resetsAtMs: NOW_MS + 130 * MINUTE_MS }],
    }
    const fresh = local({ measure, effort: 'xhigh' })
    const down = { status: 'unreachable', detail: 'ECONNREFUSED' } as const
    const lines = screen(input({ local: fresh, daemon: down }))
    expect(text(line(lines, 'headline'))).toBe('Opus 5.5 · xhigh                 $9.80')
    expect(text(line(lines, 'limit-five_hour'))).toBe('  5h    █████████████████▊░░░░░░░  71%')
    expect(statusText({ snapshot: snapshot(), local: fresh, daemon: down, nowMs: NOW_MS })).toBe('$9.80 · cache 52:14 · 5h 71%')
    expect(statusText({ snapshot: snapshot(), local: fresh, daemon: OK, nowMs: NOW_MS })).toBe('$4.18 · cache 52:14 · 5h 71%')
  })

  test('a window Claude Code says has reset reads 0% without a pace', () => {
    const measure = { ...local().measure!, atMs: NOW_MS, rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAtMs: NOW_MS - MINUTE_MS }] }
    const lines = screen(input({ local: local({ measure }) }))
    expect(text(line(lines, 'limit-five_hour'))).toBe('  5h    ░░░░░░░░░░░░░░░░░░░░░░░░░   0%')
    expect(lines.some(candidate => candidate.key === 'limit-five_hour-detail')).toBe(false)
  })

  test('greets a fresh session before the first prompt', () => {
    const fresh = { session: 's1', model: null, effort: null, turns: 0, working: false, measure: null, step: null }
    expect(frame(screen(input({ snapshot: null, local: fresh, daemon: { status: 'starting', detail: null } })))).toEqual([
      'Starting the claude-plus daemon…',
      ' ',
      'Claude',
      ' ',
      'Context',
      'Context appears after your first prompt',
      ' ',
      'Cache                                 ',
      'Cache starts with your first prompt ✻',
      ' ',
      'Session                               ',
      '  Turns                              0',
    ])
  })

  test('names the daemon problem in plain words', () => {
    const unreachable = screen(input({ daemon: { status: 'unreachable', detail: 'no answer within 500 ms' } }))
    expect(text(line(unreachable, 'daemon'))).toBe('The daemon is not answering · run claude-plus daemon --ensure')
    const rebuilding = screen(input({ daemon: { status: 'rebuilding', detail: null } }))
    expect(text(line(rebuilding, 'daemon'))).toBe('Rebuilding the claude-plus index… numbers return in a moment')
    const failed = screen(input({ daemon: { status: 'failed', detail: 'the database is locked' } }))
    expect(line(failed, 'daemon').segments[0]).toEqual({ text: 'The daemon could not start: the database is locked', color: 'error' })
  })

  test('ignores a snapshot that belongs to another session', () => {
    const lines = screen(input({ local: local({ session: 's2' }) }))
    expect(lines.some(candidate => candidate.key === 'next')).toBe(false)
    expect(text(line(lines, 'session-turns'))).toBe('  Turns                             14')
  })
})

describe('motion', () => {
  test('the heartbeat breathes over four seconds and quickens in the last minute', () => {
    const warm = cacheClock(snapshot(), local(), NOW_MS)
    const phases = [0, 1, 2, 3].map(second => heartbeat(warm, 1_000 * second, true, false))
    expect(phases).toEqual([
      { text: '✻', color: 'claude', dim: true },
      { text: '✻', color: 'claude' },
      { text: '✻', color: 'claude', bold: true },
      { text: '✻', color: 'claude' },
    ])
    const lastMinute = cacheClock(snapshot(), local(), NOW_MS + 52 * MINUTE_MS)
    expect([0, 500].map(ms => heartbeat(lastMinute, ms, true, false))).toEqual([
      { text: '✻', color: 'warning', bold: true },
      { text: '✻', color: 'warning', dim: true },
    ])
  })

  test('the heartbeat holds still without animations or while Claude is working', () => {
    const warm = cacheClock(snapshot(), local(), NOW_MS)
    expect(heartbeat(warm, 0, false, false)).toEqual({ text: '✻', color: 'claude' })
    expect(heartbeat(warm, 0, true, true)).toEqual({ text: '✻', color: 'claude' })
    expect(heartbeat(cacheClock(snapshot(), local(), NOW_MS + 53 * MINUTE_MS), 0, true, false)).toEqual({ text: '·', dim: true })
  })

  test('the cache clock follows a live request before the daemon has seen it', () => {
    const step = { atMs: NOW_MS, model: 'claude-opus-5-5', input: 1, output: 1, cacheRead: 1000, cacheWrite: 0 }
    const clock = cacheClock(snapshot(), local({ step }), NOW_MS)
    expect([clock.state, clock.remainingMs, clock.assumed]).toEqual(['warm', 60 * MINUTE_MS, false])
    const unsubscribed = cacheClock(null, local({ step, measure: null }), NOW_MS)
    expect([unsubscribed.ttlMs, unsubscribed.assumed]).toEqual([5 * MINUTE_MS, true])
  })

  test('numbers ease out over 300 ms and settle on the target', () => {
    const motion = { from: 0, to: 100, startMs: 0 }
    expect([0, 100, 150, 300, 900].map(ms => Math.round(eased(motion, ms)))).toEqual([0, 70, 88, 100, 100])
    const lines = screen(input({ motions: { cost: { from: 4, to: 4.18, startMs: NOW_MS - 100 } } }))
    expect(line(lines, 'headline').segments[1]?.text).toBe('$4.13')
  })

  test('a new value eases from wherever the old motion had reached', () => {
    const running = { cost: { from: 0, to: 10, startMs: 0 } }
    expect(retarget(running, { cost: 10, hit: 90 }, { cost: 20, hit: 90 }, 150, true)).toEqual({ cost: { from: 8.75, to: 20, startMs: 150 } })
    expect(retarget(running, { cost: 10 }, { cost: 20 }, 150, false)).toEqual({})
    expect(settled({ cost: { from: 0, to: 1, startMs: 0 } }, EASE_MS)).toEqual({})
  })

  test('the ticker runs at 20 fps only while something moves', () => {
    const base = { animations: true, motions: {}, nowMs: NOW_MS, snapshot: snapshot(), local: local() }
    expect(tickPeriod(base)).toBe(TICK_MS)
    expect(tickPeriod({ ...base, motions: { cost: { from: 0, to: 1, startMs: NOW_MS } } })).toBe(FRAME_MS)
    expect(tickPeriod({ ...base, nowMs: NOW_MS + 52 * MINUTE_MS })).toBe(QUICK_BREATH_MS)
    expect(tickPeriod({ ...base, animations: false, nowMs: NOW_MS + 52 * MINUTE_MS })).toBe(TICK_MS)
  })
})

describe('status and daemon', () => {
  test('the status line carries cost, cache countdown and the 5h window', () => {
    expect(statusText({ snapshot: snapshot(), local: local(), daemon: OK, nowMs: NOW_MS })).toBe('$4.18 · cache 52:14 · 5h 54%')
    expect(statusText({ snapshot: snapshot(), local: local(), daemon: OK, nowMs: NOW_MS + 53 * MINUTE_MS })).toBe('$4.18 · cache cold · 5h 54%')
    expect(statusText({ snapshot: null, local: { ...local(), measure: null }, daemon: OK, nowMs: NOW_MS })).toBeUndefined()
  })

  test('the ensure outcome tells a missing binary from a failed start', () => {
    expect(ensureOutcome(0, '')).toEqual({ status: 'ok', detail: null })
    expect(ensureOutcome(127, '')).toEqual({ status: 'missing', detail: null })
    expect(ensureOutcome(1, 'Error: the claude-plus daemon exited at start\n')).toEqual({ status: 'failed', detail: 'the claude-plus daemon exited at start' })
    expect(ensureError(new Error('spawn claude-plus ENOENT'))).toEqual({ status: 'missing', detail: null })
  })

  test('a question mark toggles its explanation', () => {
    expect(toggled([], 'cache')).toEqual(['cache'])
    expect(toggled(['cache', 'limits'], 'cache')).toEqual(['limits'])
  })
})
