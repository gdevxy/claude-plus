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
  nextTickDelay,
  retarget,
  screen,
  settled,
  statusText,
  tickPeriod,
  toggled,
} from '../hooks/state'
import { MINUTE_MS, NOW_MS, local, snapshot } from './fixtures'
import { overflows, painted, trimmed } from './paint'

const OK = { status: 'ok', detail: null } as const

function input(overrides: Partial<ScreenInput> = {}): ScreenInput {
  return { width: 40, nowMs: NOW_MS, animations: true, snapshot: snapshot(), local: local(), daemon: OK, expanded: [], motions: {}, ...overrides }
}

function text(line: Line): string {
  return line.segments.map(segment => segment.text).join('')
}

function line(lines: Line[], key: string): Line {
  const found = lines.find(candidate => candidate.key === key)
  if (found === undefined) throw new Error(`no line ${key}`)
  return found
}

const FRAMES: { name: string; state: Partial<ScreenInput>; rows: Record<number, string[]> }[] = [
  {
    name: 'the whole session',
    state: {},
    rows: {
      40: [
        ' Opus 5.5 · high                  $4.18',
        '',
        ' Context                     46k / 200k',
        ' ███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (1h)         52:14 left ?',
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
        '         resets 2h 10m · pace 81%',
        '   7d    ███████▊░░░░░░░░░░░░░░░░░  31%',
        '         resets 3d',
      ],
      60: [
        ' Opus 5.5 · high                                      $4.18',
        '',
        ' Context                                         46k / 200k',
        ' ████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (1h)                             52:14 left ?',
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
        '         resets in 2h 10m · on pace for 81%',
        '   7d    ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
        '         resets in 3d',
      ],
      100: [
        ' Opus 5.5 · high                                                                              $4.18',
        '',
        ' Context                                                                                 46k / 200k',
        ' █████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (1h)                                                                     52:14 left ?',
        ' █████████████████████████████████████████████████████████████████████████████████████▍░░░░░░░░░░░░',
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
        '         resets in 2h 10m · on pace for 81%',
        '   7d    ██████████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
        '         resets in 3d',
      ],
    },
  },
  {
    name: 'Claude Code alone while the daemon is missing',
    state: { snapshot: null, local: local({ step: { atMs: NOW_MS - 1000, model: 'claude-opus-5-5', input: 12, output: 800, cacheRead: 25_000, cacheWrite: 300 } }), daemon: { status: 'missing', detail: null } },
    rows: {
      40: [
        ' claude-plus is not installed yet.',
        ' Install it, or set its path in the',
        ' plugin options',
        '',
        ' Opus 5.5                         $4.18',
        '',
        ' Context                     46k / 200k',
        ' ███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (~1h)        59:59 left ?',
        ' ██████████████████████████████████████',
        '',
        ' Session',
        '   Turns                             14',
        '   Last request      25.3k in · 800 out',
        '',
        ' Plan limits                          ?',
        '   5h    █████████████▌░░░░░░░░░░░  54%',
        '         resets in 2h 10m',
      ],
      60: [
        ' claude-plus is not installed yet. Install it, or set its',
        ' path in the plugin options',
        '',
        ' Opus 5.5                                             $4.18',
        '',
        ' Context                                         46k / 200k',
        ' ████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (~1h)                            59:59 left ?',
        ' ██████████████████████████████████████████████████████████',
        '',
        ' Session',
        '   Turns                                                 14',
        '   Last request                          25.3k in · 800 out',
        '',
        ' Plan limits                                              ?',
        '   5h    ████████████████████████▎░░░░░░░░░░░░░░░░░░░░  54%',
        '         resets in 2h 10m',
      ],
      100: [
        ' claude-plus is not installed yet. Install it, or set its path in the plugin options',
        '',
        ' Opus 5.5                                                                                     $4.18',
        '',
        ' Context                                                                                 46k / 200k',
        ' █████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache ✻ warm (~1h)                                                                    59:59 left ?',
        ' ██████████████████████████████████████████████████████████████████████████████████████████████████',
        '',
        ' Session',
        '   Turns                                                                                         14',
        '   Last request                                                                  25.3k in · 800 out',
        '',
        ' Plan limits                                                                                      ?',
        '   5h    █████████████████████████████████████████████▉░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  54%',
        '         resets in 2h 10m',
      ],
    },
  },
  {
    name: 'a fresh session before the first prompt',
    state: { snapshot: null, local: { session: 's1', model: null, effort: null, turns: 0, working: false, measure: null, step: null, breakdown: null }, daemon: { status: 'starting', detail: null } },
    rows: {
      40: [
        ' Starting the claude-plus daemon…',
        '',
        ' Claude',
        '',
        ' Context',
        ' Context shows after your first prompt',
        '',
        ' Cache',
        ' Cache starts with your first prompt ✻',
        '',
        ' Session',
        '   Turns                              0',
      ],
      60: [
        ' Starting the claude-plus daemon…',
        '',
        ' Claude',
        '',
        ' Context',
        ' Context shows after your first prompt',
        '',
        ' Cache',
        ' Cache starts with your first prompt ✻',
        '',
        ' Session',
        '   Turns                                                  0',
      ],
      100: [
        ' Starting the claude-plus daemon…',
        '',
        ' Claude',
        '',
        ' Context',
        ' Context shows after your first prompt',
        '',
        ' Cache',
        ' Cache starts with your first prompt ✻',
        '',
        ' Session',
        '   Turns                                                                                          0',
      ],
    },
  },
  {
    name: 'a cache that went cold',
    state: { nowMs: NOW_MS + 53 * MINUTE_MS },
    rows: {
      40: [
        ' Opus 5.5 · high                  $4.18',
        '',
        ' Context                     46k / 200k',
        ' ███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache · cold (1h)            expired ?',
        ' ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░',
        ' Cache went cold while you were away',
        '   Rewarming will cost           ~$0.40',
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
        '         resets 1h 17m · pace 81%',
        '   7d    ███████▊░░░░░░░░░░░░░░░░░  31%',
        '         resets 2d 23h',
      ],
      60: [
        ' Opus 5.5 · high                                      $4.18',
        '',
        ' Context                                         46k / 200k',
        ' ████████████▎░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache · cold (1h)                                expired ?',
        ' ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░',
        ' Cache went cold while you were away',
        '   Rewarming will cost                               ~$0.40',
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
        '         resets in 1h 17m · on pace for 81%',
        '   7d    ██████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
        '         resets in 2d 23h',
      ],
      100: [
        ' Opus 5.5 · high                                                                              $4.18',
        '',
        ' Context                                                                                 46k / 200k',
        ' █████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
        '',
        ' Cache · cold (1h)                                                                        expired ?',
        ' ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░',
        ' Cache went cold while you were away, rewarming will cost ~$0.40',
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
        '         resets in 1h 17m · on pace for 81%',
        '   7d    ██████████████████████████▍░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  31%',
        '         resets in 2d 23h',
      ],
    },
  },
]

describe('frames', () => {
  for (const frame of FRAMES) {
    for (const width of [40, 60, 100]) {
      test(`draws ${frame.name} at ${width} columns`, () => {
        const rows = painted(screen(input({ width, ...frame.state })), width)
        expect(rows.filter(row => row.length > width)).toEqual([])
        expect(trimmed(rows)).toEqual(frame.rows[width])
      })
    }
  }
})

describe('live screen', () => {
  for (const width of [40, 60, 100]) {
    test(`every unwrapped line fills exactly ${width - 2} columns`, () => {
      const lines = screen(input({ width, expanded: ['cache', 'rebuilds', 'forecast', 'limits'] }))
      for (const drawn of lines.filter(candidate => candidate.wrap !== true && !candidate.key.startsWith('gap-'))) {
        expect([drawn.key, (drawn.indent ?? 0) + text(drawn).length + (drawn.why === undefined ? 0 : WHY_COLUMNS)]).toEqual([drawn.key, width - 2])
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
    expect(line(cold, 'cache').segments[3]).toEqual({ text: 'expired', dim: true })
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
      'Your 46k token prompt is read from the cache when warm and written again when cold. 2k new and 600 output tokens are typical of your last 10 requests.',
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
      segments: [{ text: 'resets 2h 10m · full in 54m'.padEnd(30), color: 'warning' }],
      indent: 8,
    })
    expect(text(line(screen(input({ width: 100, snapshot: snapshot({ limits: [filling, seven!] }) })), 'limit-five_hour-detail')).trimEnd()).toBe(
      'resets in 2h 10m · full in 54m at this pace',
    )
  })

  test('says it is still learning until the plan has enough readings', () => {
    const [five] = snapshot().live.limits
    const learning = { ...five!, calibration: { family: 'opus', pairs: 3, needed: 20, pct_per_usd: null } }
    const lines = screen(input({ snapshot: snapshot({ limits: [learning] }), expanded: ['limits'] }))
    expect([text(line(lines, 'limits-learning')), text(line(lines, 'limits-learning-count'))]).toEqual(['Learning how your plan counts tokens…', '3 of 20 readings ✻'])
    expect(text(line(lines, 'explain-limits'))).toBe(
      'Each turn pairs the percent Claude Code reports with what it would cost on the API; 3 of 20 readings so far. The pace projects the last 30 minutes to the reset.',
    )
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

  test('names the daemon problem in plain words', () => {
    const unreachable = screen(input({ daemon: { status: 'unreachable', detail: 'no answer within 500 ms' } }))
    expect([text(line(unreachable, 'daemon')), text(line(unreachable, 'daemon-fix'))]).toEqual(['The daemon is not answering', 'Run claude-plus daemon --ensure'])
    const wide = screen(input({ width: 100, daemon: { status: 'unreachable', detail: 'no answer within 500 ms' } }))
    expect(text(line(wide, 'daemon'))).toBe('The daemon is not answering · run claude-plus daemon --ensure')
    const outdated = screen(input({ daemon: { status: 'outdated', detail: null } }))
    expect(text(line(outdated, 'daemon'))).toBe('Restarting the out-of-date claude-plus daemon…')
    const behind = screen(input({ width: 100, daemon: { status: 'behind', detail: null } }))
    expect(text(line(behind, 'daemon'))).toBe('The claude-plus daemon is older than this pane · update the claude-plus binary')
    const newer = screen(input({ width: 100, daemon: { status: 'newer', detail: null } }))
    expect(text(line(newer, 'daemon'))).toBe('The claude-plus daemon is newer than this pane · update the plus plugin')
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

const FILLING = (): ScreenInput['snapshot'] => {
  const [five, seven] = snapshot().live.limits
  return snapshot({ limits: [{ ...five!, used_pct: 82, pace: { pct_per_hour: 20, projected_pct: 125, full_at_ms: NOW_MS + 54 * MINUTE_MS } }, seven!] })
}

const LEARNING = (): ScreenInput['snapshot'] => {
  const [five] = snapshot().live.limits
  return snapshot({ limits: [{ ...five!, calibration: { family: 'opus', pairs: 3, needed: 20, pct_per_usd: null } }] })
}

const STATES: Record<string, Partial<ScreenInput>> = {
  'the whole session': { expanded: ['cache', 'rebuilds', 'forecast', 'limits'] },
  'a cold cache': { nowMs: NOW_MS + 53 * MINUTE_MS },
  'a fresh session': { snapshot: null, local: { ...local(), model: null, turns: 0, measure: null }, daemon: { status: 'starting', detail: null } },
  'a missing daemon': { snapshot: null, daemon: { status: 'missing', detail: null } },
  'an unreachable daemon': { daemon: { status: 'unreachable', detail: 'ECONNREFUSED' } },
  'a filling window': { snapshot: FILLING() },
  'a plan still learning': { snapshot: LEARNING() },
  'an older daemon': { daemon: { status: 'outdated', detail: null } },
  'a newer daemon': { daemon: { status: 'newer', detail: null } },
  'an old binary': { daemon: { status: 'behind', detail: null } },
  'unpriced requests': { snapshot: { ...snapshot(), totals: { ...snapshot().totals, unpriced_requests: 2 } } },
}

describe('fitting', () => {
  for (const width of [40, 60, 100]) {
    for (const [name, state] of Object.entries(STATES)) {
      test(`${name} at ${width} columns cuts no row and splits no value`, () => {
        expect(overflows(screen(input({ width, ...state })), width)).toEqual([])
      })
    }
  }

  test('the cache line keeps one separator, cold or warm', () => {
    expect(text(line(screen(input()), 'cache'))).toBe('Cache ✻ warm (1h)         52:14 left')
    expect(text(line(screen(input({ nowMs: NOW_MS + 53 * MINUTE_MS })), 'cache'))).toBe('Cache · cold (1h)            expired')
  })

  test('a nearly full context warns without calling it an error', () => {
    const lines = screen(input({ local: local({ measure: { ...local().measure!, contextPercent: 96 } }) }))
    expect(line(lines, 'context-bar').segments[1]?.color).toBe('warning')
  })

  test('the plan learning note counts its readings', () => {
    const lines = screen(input({ width: 100, snapshot: LEARNING() }))
    expect(text(line(lines, 'limits-learning'))).toBe('Learning how your plan counts tokens… 3 of 20 readings ✻')
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

  test('the next tick lands on the second the countdown changes', () => {
    expect([nextTickDelay(1000, 5300, 5000), nextTickDelay(1000, 5300, 5300), nextTickDelay(1000, 0, 5450), nextTickDelay(500, 5300, 5100)]).toEqual([300, 1000, 550, 200])
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
