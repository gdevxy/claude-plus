import { describe, expect, test } from 'claude-code/testing'

import type { Line, ScreenInput } from '../hooks/state'
import type { Local } from '../types'
import {
  EASE_MS,
  FRAME_MS,
  QUICK_BREATH_MS,
  TICK_MS,
  WHY_COLUMNS,
  breath,
  cacheClock,
  eased,
  ensureError,
  ensureOutcome,
  heartbeat,
  limitsOf,
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

function capped(breakdownModel: string): Local {
  const measure = { ...local().measure!, contextTokens: 51_200, contextWindow: 1_000_000, contextPercent: 5 }
  const breakdown = { ts_ms: NOW_MS, model: breakdownModel, total_tokens: 51_200, max_tokens: 400_000, percentage: 13, auto_compact_threshold: 367_000, categories: [] }
  return local({ model: 'claude-sonnet-5-5[1m]', measure, breakdown })
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
        ' Next prompt · cache warm             ?',
        '   Fable 5.1        ~$0.67, then ~$0.05',
        ' ● Opus 5.5        ~$0.03 · <0.1% of 5h',
        '     if it expires  ~$0.40 · 0.2% of 5h',
        '   Sonnet 5.5       ~$0.20, then ~$0.02',
        '   Haiku 5.5        ~$0.07, then <$0.01',
        '',
        ' Session                              ?',
        '   Processed / generated       2M / 61k',
        '   Turns · subagents             14 · 2',
        '',
        ' Limits                               ?',
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
        ' Next prompt · cache warm                                 ?',
        '   Fable 5.1                            ~$0.67, then ~$0.05',
        ' ● Opus 5.5                            ~$0.03 · <0.1% of 5h',
        '     if the cache expires               ~$0.40 · 0.2% of 5h',
        '   Sonnet 5.5                           ~$0.20, then ~$0.02',
        '   Haiku 5.5                            ~$0.07, then <$0.01',
        '',
        ' Session                                                  ?',
        '   Processed / generated                           2M / 61k',
        '   Turns · subagents                                 14 · 2',
        '',
        ' Limits                                                   ?',
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
        ' Next prompt · cache warm                                                                         ?',
        '   Fable 5.1                                                                    ~$0.67, then ~$0.05',
        ' ● Opus 5.5                                                                    ~$0.03 · <0.1% of 5h',
        '     if the cache expires                                                       ~$0.40 · 0.2% of 5h',
        '   Sonnet 5.5                                                                   ~$0.20, then ~$0.02',
        '   Haiku 5.5                                                                    ~$0.07, then <$0.01',
        '',
        ' Session                                                                                          ?',
        '   Processed / generated                                                                   2M / 61k',
        '   Turns · subagents                                                                         14 · 2',
        '',
        ' Limits                                                                                           ?',
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
        ' Limits                               ?',
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
        ' Limits                                                   ?',
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
        ' Limits                                                                                           ?',
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
        ' Next prompt · cache cold             ?',
        '   Fable 5.1        ~$0.67, then ~$0.05',
        ' ● Opus 5.5         ~$0.40 · 0.2% of 5h',
        '     then          ~$0.03 · <0.1% of 5h',
        '   Sonnet 5.5       ~$0.20, then ~$0.02',
        '   Haiku 5.5        ~$0.07, then <$0.01',
        '',
        ' Session                              ?',
        '   Processed / generated       2M / 61k',
        '   Turns · subagents             14 · 2',
        '',
        ' Limits                               ?',
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
        ' Next prompt · cache cold                                 ?',
        '   Fable 5.1                            ~$0.67, then ~$0.05',
        ' ● Opus 5.5                             ~$0.40 · 0.2% of 5h',
        '     then                              ~$0.03 · <0.1% of 5h',
        '   Sonnet 5.5                           ~$0.20, then ~$0.02',
        '   Haiku 5.5                            ~$0.07, then <$0.01',
        '',
        ' Session                                                  ?',
        '   Processed / generated                           2M / 61k',
        '   Turns · subagents                                 14 · 2',
        '',
        ' Limits                                                   ?',
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
        ' Next prompt · cache cold                                                                         ?',
        '   Fable 5.1                                                                    ~$0.67, then ~$0.05',
        ' ● Opus 5.5                                                                     ~$0.40 · 0.2% of 5h',
        '     then                                                                      ~$0.03 · <0.1% of 5h',
        '   Sonnet 5.5                                                                   ~$0.20, then ~$0.02',
        '   Haiku 5.5                                                                    ~$0.07, then <$0.01',
        '',
        ' Session                                                                                          ?',
        '   Processed / generated                                                                   2M / 61k',
        '   Turns · subagents                                                                         14 · 2',
        '',
        ' Limits                                                                                           ?',
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
      const lines = screen(input({ width, expanded: ['cache', 'rebuilds', 'forecast', 'session', 'limits'] }))
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
    expect(line(lines, 'cache').segments[1]).toEqual({ text: '✻', color: 'claude', glow: 1 })
    expect(line(lines, 'cache').segments[3]).toEqual({ text: '52:14 left', color: 'success' })
    expect(line(lines, 'cache-bar').segments[0]?.color).toBe('success')
  })

  test('measures the context against a capped compaction window and notes the model window', () => {
    const lines = screen(input({ width: 60, local: capped('claude-sonnet-5-5') }))
    expect(text(line(lines, 'context'))).toBe(`Context${' '.repeat(58 - 7 - 23)}51.2k / 400k · 1M model`)
    expect(line(lines, 'context').segments.at(-1)).toEqual({ text: ' · 1M model', dim: true })
    const bar = text(line(lines, 'context-bar'))
    expect(bar.endsWith(' 13%')).toBe(true)
    expect(bar.indexOf('│')).toBe(Math.floor((367_000 / 400_000) * 53))
  })

  test('drops the model window note when the line is too narrow', () => {
    expect(text(line(screen(input({ width: 26, local: capped('claude-sonnet-5-5') })), 'context'))).toBe('Context     51.2k / 400k')
  })

  test('ignores a breakdown counted on another model', () => {
    const lines = screen(input({ width: 60, local: capped('claude-opus-5-5') }))
    expect(text(line(lines, 'context'))).toBe(`Context${' '.repeat(58 - 7 - 10)}51.2k / 1M`)
    expect(text(line(lines, 'context-bar')).endsWith('  5%')).toBe(true)
    expect(text(line(lines, 'context-bar'))).not.toContain('│')
  })

  test('marks the auto-compact point on an uncapped window until the bar passes it', () => {
    const breakdown = { ts_ms: NOW_MS, model: 'claude-opus-5-5', total_tokens: 46_000, max_tokens: 200_000, percentage: 23, auto_compact_threshold: 167_000, categories: [] }
    const lines = screen(input({ local: local({ breakdown }) }))
    expect(text(line(lines, 'context'))).toBe('Context                     46k / 200k')
    expect(text(line(lines, 'context-bar')).indexOf('│')).toBe(Math.floor((167_000 / 200_000) * 33))
    const full = local({ breakdown, measure: { ...local().measure!, contextTokens: 180_000, contextPercent: 90 } })
    expect(text(line(screen(input({ local: full })), 'context-bar'))).not.toContain('│')
  })

  test('turns the countdown amber in its last minute and cold after it', () => {
    const urgent = screen(input({ nowMs: NOW_MS + 51 * MINUTE_MS + 30_000 }))
    expect(line(urgent, 'cache').segments[3]).toEqual({ text: '0:44 left', color: 'warning' })
    const cold = screen(input({ nowMs: NOW_MS + 53 * MINUTE_MS }))
    expect(line(cold, 'cache').segments[3]).toEqual({ text: 'expired', dim: true })
    expect(text(line(cold, 'next'))).toBe('Next prompt · cache cold'.padEnd(36))
    expect(text(line(cold, 'next-current'))).toMatch(/^● Opus 5\.5 +~\$0\.40 · 0\.2% of 5h$/)
    expect(text(line(cold, 'next-after'))).toMatch(/^ {4}then +~\$0\.03 · <0\.1% of 5h$/)
  })

  test('marks the current model in the lineup and says what the next prompt costs on each', () => {
    const lines = screen(input())
    expect(line(lines, 'next').segments).toEqual([
      { text: 'Next prompt', bold: true },
      { text: ' · cache warm'.padEnd(25), dim: true },
    ])
    const current = line(lines, 'next-current').segments
    expect(current[0]).toEqual({ text: '● ', color: 'claude' })
    expect([current[1]?.bold, current[2]]).toEqual([true, { text: '~$0.03 · <0.1% of 5h', bold: true }])
    expect(line(lines, 'next-after').segments.every(segment => segment.dim === true)).toBe(true)
    expect(text(line(lines, 'next-claude-haiku-5-5'))).toMatch(/^ {2}Haiku 5\.5 +~\$0\.07, then <\$0\.01$/)
  })

  test('names what changed when the injected context rewrote the cache', () => {
    const cache = snapshot().live.cache
    const changed = snapshot({ cache: { ...cache, last_rebuild: { ts_ms: NOW_MS - 40 * MINUTE_MS, cause: 'prefix_changed', tokens: 51_000, extra_usd: 0.4 } } })
    expect(text(line(screen(input({ snapshot: changed, expanded: ['rebuilds'] })), 'explain-rebuilds'))).toBe(
      'The last rebuild, 40m ago, wrote the prompt again because the MCP tools, CLAUDE.md, skills or agents changed.',
    )
  })

  test('explains each number once its question mark is pressed', () => {
    const lines = screen(input({ expanded: ['cache', 'rebuilds', 'forecast', 'session', 'limits'] }))
    expect(text(line(lines, 'explain-cache'))).toBe(
      'The last cache write in the transcript asked for a 1 hour lifetime. Every request that reads or writes the cache starts the countdown again.',
    )
    expect(text(line(lines, 'explain-rebuilds'))).toBe(
      'The last rebuild, 40m ago, wrote the prompt again because the cache expired during an idle gap.',
    )
    expect(text(line(lines, 'explain-forecast'))).toBe(
      'Your 46k token prompt is read from the cache when warm and written again when cold. Each model keeps its own cache, so switching starts cold. 2k new and 600 output tokens are typical of your last 10 requests.',
    )
    expect(text(line(lines, 'explain-limits'))).toBe(
      'From 23 readings, 1% of your 5h window is about $2.00 of API-priced usage on Opus. The pace projects the last 30 minutes to the reset.',
    )
    expect(text(line(lines, 'explain-session'))).toBe(
      'Every request sends the whole context again, so processed sums the context of each request, mostly as cheap cache reads. Generated counts what the model wrote.',
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
    const stopped = screen(input({ daemon: { status: 'stopped', detail: null } }))
    expect([text(line(stopped, 'daemon')), text(line(stopped, 'daemon-fix'))]).toEqual(['The claude-plus daemon is stopped', 'Run /plus to start it'])
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
  'the whole session': { expanded: ['cache', 'rebuilds', 'forecast', 'session', 'limits'] },
  'a cold cache': { nowMs: NOW_MS + 53 * MINUTE_MS },
  'a fresh session': { snapshot: null, local: { ...local(), model: null, turns: 0, measure: null }, daemon: { status: 'starting', detail: null } },
  'a missing daemon': { snapshot: null, daemon: { status: 'missing', detail: null } },
  'an unreachable daemon': { daemon: { status: 'unreachable', detail: 'ECONNREFUSED' } },
  'a stopped daemon': { daemon: { status: 'stopped', detail: null } },
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

  for (let width = 26; width <= 32; width += 1) {
    for (const [name, state] of Object.entries(STATES)) {
      test(`${name} at ${width} columns cuts no label mid-word`, () => {
        const lines = screen(input({ width, ...state }))
        expect(overflows(lines, width).filter(problem => problem.includes(' is cut at '))).toEqual([])
        expect(painted(lines, width).filter(row => /…(?!\s*✻?\s*$)/.test(row))).toEqual([])
      })
    }
  }

  test('label and value rows fall back to shorter whole words at the narrowest width', () => {
    const rows = (state: Partial<ScreenInput>) => trimmed(painted(screen(input({ width: 26, ...state })), 26))
    const warm = rows({})
    expect(warm.filter(row => /Cache ✻|Fable|● Opus|expires|Sonnet|Haiku|In \/ out|Turns/.test(row))).toEqual([
      ' Cache ✻     52:14 left ?',
      '   Fable 5.1       ~$0.67',
      ' ● Opus 5.5        ~$0.03',
      '     if it expires ~$0.40',
      '   Sonnet 5.5      ~$0.20',
      '   Haiku 5.5       ~$0.07',
      '   In / out      2M / 61k',
      '   Turns · agents  14 · 2',
    ])
    const cold = rows({ nowMs: NOW_MS + 53 * MINUTE_MS })
    expect(cold.filter(row => /Cache ·|Rewarming|● Opus|then/.test(row))).toEqual([
      ' Cache · cold   expired ?',
      '   Rewarming costs ~$0.40',
      ' ● Opus 5.5        ~$0.40',
      '     then          ~$0.03',
    ])
    expect(rows({}).filter(row => row.includes('<0.1% of 5h'))).toEqual([])
    expect(trimmed(painted(screen(input({ width: 40 })), 40)).filter(row => /● Opus|expires|Fable/.test(row))).toEqual([
      '   Fable 5.1        ~$0.67, then ~$0.05',
      ' ● Opus 5.5        ~$0.03 · <0.1% of 5h',
      '     if it expires  ~$0.40 · 0.2% of 5h',
    ])
  })

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
  test('the heartbeat breathes in and out over four seconds and quickens to one in the last minute', () => {
    const warm = cacheClock(snapshot(), local(), NOW_MS)
    expect([0, 500, 1000, 2000, 3000, 4000].map(ms => heartbeat(warm, NOW_MS + ms, true, false))).toEqual(
      [1, 0.854, 0.5, 0, 0.5, 1].map(glow => ({ text: '✻', color: 'claude', glow })),
    )
    const lastMinute = cacheClock(snapshot(), local(), NOW_MS + 52 * MINUTE_MS)
    expect([0, 250, 500].map(ms => heartbeat(lastMinute, NOW_MS + 52 * MINUTE_MS + ms, true, false))).toEqual(
      [0, 0.5, 1].map(glow => ({ text: '✻', color: 'warning', glow })),
    )
  })

  test('a breath is a smooth wave, phased to the cache expiry', () => {
    expect([-4000, -3000, -1000, 0, 1000, 2000].map(ms => breath(ms, 4000))).toEqual([0, 0.5, 0.5, 0, 0.5, 1])
    const samples = Array.from({ length: 41 }, (_, index) => breath(index * 100, 4000))
    expect(Math.max(...samples.slice(1).map((glow, index) => Math.abs(glow - (samples[index] ?? 0))))).toBeLessThan(0.08)
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

  test('the ticker runs at 20 fps only while something moves, and steps the heartbeat itself only when nothing blits it', () => {
    const base = { animations: true, motions: {}, nowMs: NOW_MS, snapshot: snapshot(), local: local(), breathing: false }
    expect(tickPeriod(base)).toBe(TICK_MS)
    expect(tickPeriod({ ...base, motions: { cost: { from: 0, to: 1, startMs: NOW_MS } } })).toBe(FRAME_MS)
    expect(tickPeriod({ ...base, nowMs: NOW_MS + 52 * MINUTE_MS })).toBe(QUICK_BREATH_MS / 2)
    expect(tickPeriod({ ...base, animations: false, nowMs: NOW_MS + 52 * MINUTE_MS })).toBe(TICK_MS)
    expect(tickPeriod({ ...base, breathing: true, nowMs: NOW_MS + 52 * MINUTE_MS })).toBe(TICK_MS)
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
    expect(ensureOutcome(3, '')).toEqual({ status: 'stopped', detail: null })
    expect(ensureOutcome(1, 'Error: the claude-plus daemon exited at start\n')).toEqual({ status: 'failed', detail: 'the claude-plus daemon exited at start' })
    expect(ensureError(new Error('spawn claude-plus ENOENT'))).toEqual({ status: 'missing', detail: null })
  })

  test('a question mark toggles its explanation', () => {
    expect(toggled([], 'cache')).toEqual(['cache'])
    expect(toggled(['cache', 'limits'], 'cache')).toEqual(['limits'])
  })
})

describe('accounts and budgets', () => {
  const enterprise = { id: 'oauth:work', label: 'Acme · Enterprise', kind: 'oauth' as const }
  const budget = { amount_usd: 700, local_usd: 23.1, period_start_ms: Date.parse('2026-10-01T00:00:00Z'), resets_at_ms: Date.parse('2026-11-01T00:00:00Z') }
  const spend = (used: number) => ({
    window: 'spend_limit',
    used_pct: used,
    resets_at_ms: budget.resets_at_ms,
    sampled_at_ms: NOW_MS - MINUTE_MS,
    reset_since_sample: false,
    pace: { pct_per_hour: 0.01, projected_pct: 31, full_at_ms: null },
    calibration: { family: 'opus', pairs: 0, needed: 20, pct_per_usd: null },
  })
  const own = () => ({ ...spend(3.3), window: 'budget' })
  const [five] = snapshot().live.limits
  const alone = local({ measure: { ...local().measure!, rateLimits: [] } })

  test('names the account under the headline', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, limits: [] }) }))
    expect(text(line(lines, 'account')).trim()).toBe('Acme · Enterprise')
    expect(line(lines, 'account').segments[0]?.dim).toBe(true)
    expect(lines.findIndex(candidate => candidate.key === 'account')).toBe(lines.findIndex(candidate => candidate.key === 'headline') + 1)
  })

  test('cuts a long account name with an ellipsis instead of wrapping it', () => {
    const long = { ...enterprise, label: 'Acme Corporation International · Enterprise' }
    const lines = screen(input({ width: 26, snapshot: snapshot({ account: long, limits: [] }) }))
    expect(text(line(lines, 'account'))).toBe('Acme Corporation Intern…')
  })

  test('shows the spend limit in dollars when a budget is set', () => {
    const lines = screen(input({ width: 100, snapshot: snapshot({ account: enterprise, budget, limits: [spend(4)] }) }))
    expect(text(line(lines, 'limits')).trim().startsWith('Limits')).toBe(true)
    expect(text(line(lines, 'limit-spend_limit-detail')).trim()).toBe('$28.00 of $700 · resets in 20d 15h · on pace for 31%')
    expect(text(line(lines, 'limit-spend_limit-here')).trim()).toBe('Claude Code here: $23.10')
    expect(line(lines, 'limit-spend_limit-here').segments[0]?.dim).toBe(true)
  })

  test('shortens the dollar detail as the pane narrows without cutting a word', () => {
    const detail = (width: number) => {
      const lines = screen(input({ width, snapshot: snapshot({ account: enterprise, budget, limits: [spend(4)] }) }))
      return [text(line(lines, 'limit-spend_limit-detail')).trim(), text(line(lines, 'limit-spend_limit-here')).trim()]
    }
    expect(detail(60)).toEqual(['$28 of $700 · resets 20d 15h · pace 31%', 'Claude Code here: $23.10'])
    expect(detail(40)).toEqual(['$28 of $700 · 20d 15h · 31%', 'Claude Code here: $23.10'])
    expect(detail(26)).toEqual(['$28 of $700', 'Here: $23.10'])
  })

  test('hints at the budget command when no budget is set', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, limits: [spend(4)] }) }))
    expect(text(line(lines, 'limit-spend_limit-detail')).trim()).toBe('resets in 20d 15h · on pace for 31%')
    expect(text(line(lines, 'limit-spend_limit-hint')).trim()).toBe('claude-plus budget set <usd> shows dollars')
    expect(lines.some(candidate => candidate.key === 'limit-spend_limit-here')).toBe(false)
  })

  test('an exceeded spend limit draws full with its real percent', () => {
    const lines = screen(input({ width: 100, snapshot: snapshot({ account: enterprise, budget, limits: [spend(104)] }) }))
    const meter = line(lines, 'limit-spend_limit')
    expect(text(meter)).toContain('104%')
    expect(text(meter)).not.toContain('░')
    expect(meter.segments[1]?.color).toBe('warning')
    expect(text(line(lines, 'limit-spend_limit-detail')).trim()).toBe('$28 over · resets in 20d 15h')
  })

  test('draws a budget window as this machine only and aligns its label', () => {
    const lines = screen(input({ width: 100, local: alone, snapshot: snapshot({ account: enterprise, budget, limits: [own()] }) }))
    expect(text(line(lines, 'limit-budget')).startsWith('  Budget ')).toBe(true)
    expect(text(line(lines, 'limit-budget-detail')).trim()).toBe('$23.10 of $700 here · resets in 20d 15h · on pace for 31%')
    expect(lines.some(candidate => candidate.key === 'limit-budget-here' || candidate.key === 'limits-learning')).toBe(false)
    const narrow = (width: number) => text(line(screen(input({ width, local: alone, snapshot: snapshot({ budget, limits: [own()] }) })), 'limit-budget-detail')).trim()
    expect([narrow(60), narrow(40), narrow(26)]).toEqual(['$23 of $700 here · resets 20d 15h · pace 31%', '$23 of $700 here', '$23/$700 here'])
  })

  test('a budget keeps saying here at the narrowest pane', () => {
    const narrow = (local_usd: number, amount_usd: number) =>
      text(line(screen(input({ width: 26, local: alone, snapshot: snapshot({ budget: { ...budget, local_usd, amount_usd }, limits: [own()] }) })), 'limit-budget-detail')).trim()
    expect(narrow(23.1, 700)).toBe('$23/$700 here')
    expect(narrow(1_234.5, 12_500)).toBe('$1.2k/$12.5k')
  })

  const usedUp = (used: number) => ({ ...spend(used), pace: { pct_per_hour: 0.4, projected_pct: 150, full_at_ms: null } })

  test('a spend limit past its amount says how much over instead of a pace', () => {
    const detail = (width: number) => line(screen(input({ width, snapshot: snapshot({ account: enterprise, budget, limits: [usedUp(104)] }) })), 'limit-spend_limit-detail')
    expect([100, 60, 40, 26].map(width => text(detail(width)).trim())).toEqual(['$28 over · resets in 20d 15h', '$28 over · resets in 20d 15h', '$28 over · resets in 20d 15h', '$28 over'])
    expect(detail(60).segments[0]?.color).toBe('warning')
    const atAmount = text(line(screen(input({ width: 60, snapshot: snapshot({ account: enterprise, budget, limits: [usedUp(100)] }) })), 'limit-spend_limit-detail')).trim()
    expect(atAmount).toBe('$700.00 of $700 · resets in 20d 15h')
  })

  test('a used up window without dollars shows only its reset', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ account: enterprise, limits: [usedUp(104)] }) }))
    expect(text(line(lines, 'limit-spend_limit-detail')).trim()).toBe('resets in 20d 15h')
  })

  test('a budget past its amount says how much over here', () => {
    const over = { ...own(), used_pct: 104, pace: { pct_per_hour: 0.4, projected_pct: 150, full_at_ms: null } }
    const detail = (width: number) => text(line(screen(input({ width, local: alone, snapshot: snapshot({ budget: { ...budget, local_usd: 728 }, limits: [over] }) })), 'limit-budget-detail')).trim()
    expect([detail(60), detail(26)]).toEqual(['$28 over here · resets in 20d 15h', '$28 over here'])
  })

  test('a measured spend limit replaces the daemon budget window', () => {
    const measured = local({ measure: { ...local().measure!, rateLimits: [{ kind: 'spend_limit', percentUsed: 5, resetsAtMs: budget.resets_at_ms }] } })
    const windows = limitsOf({ nowMs: NOW_MS, daemon: OK, snapshot: snapshot({ budget, limits: [own()] }), local: measured })
    expect(windows.map(window => window.window)).toEqual(['spend_limit'])
    const kept = limitsOf({ nowMs: NOW_MS, daemon: OK, snapshot: snapshot({ budget, limits: [own(), spend(4)] }), local: alone })
    expect(kept.map(window => window.window)).toEqual(['spend_limit'])
    expect(limitsOf({ nowMs: NOW_MS, daemon: OK, snapshot: snapshot({ budget, limits: [own()] }), local: alone }).map(window => window.window)).toEqual(['budget'])
  })

  test('pads every meter label to the longest window shown', () => {
    const lines = screen(input({ width: 60, snapshot: snapshot({ budget, limits: [five!, own()] }) }))
    const bars = [text(line(lines, 'limit-five_hour')), text(line(lines, 'limit-budget'))]
    expect(bars.map(bar => bar.search(/[█░▏▎▍▌▋▊▉]/))).toEqual([9, 9])
    expect(line(lines, 'limit-budget-detail').indent).toBe(9)
    expect(text(line(screen(input({ width: 60 })), 'limit-five_hour')).search(/[█░▏▎▍▌▋▊▉]/)).toBe(8)
  })

  test('says the cache lifetime is assumed from the account', () => {
    const step = { atMs: NOW_MS, model: 'claude-opus-5-5', input: 1, output: 1, cacheRead: 1000, cacheWrite: 0 }
    const lines = screen(input({ width: 60, expanded: ['cache'], snapshot: null, local: local({ step }) }))
    expect(text(line(lines, 'explain-cache'))).toContain('assumed from your account')
  })

  test('explains a monthly pace when there is no 5h window', () => {
    const lines = screen(input({ local: alone, snapshot: snapshot({ budget, limits: [own()] }), expanded: ['limits'] }))
    expect(text(line(lines, 'explain-limits'))).toBe(
      'The budget counts what Claude Code spent on this machine since the period started, at API prices. The pace projects the period so far to the reset.',
    )
  })

  const WITH_MONEY: Record<string, Partial<ScreenInput>> = {
    'a named account': { snapshot: snapshot({ account: { ...enterprise, label: 'Acme Corporation International · Enterprise' } }) },
    'a spend limit with a budget': { snapshot: snapshot({ account: enterprise, budget, limits: [...snapshot().live.limits, spend(4)] }), expanded: ['limits'] },
    'a spend limit without a budget': { snapshot: snapshot({ account: enterprise, limits: [...snapshot().live.limits, spend(4)] }) },
    'an exceeded spend limit': { local: alone, snapshot: snapshot({ account: enterprise, budget: { ...budget, amount_usd: 12_500, local_usd: 1_234.5 }, limits: [spend(104)] }) },
    'a budget': { local: alone, snapshot: snapshot({ account: enterprise, budget, limits: [own()] }), expanded: ['limits'] },
    'a used up spend limit': { local: alone, snapshot: snapshot({ account: enterprise, budget, limits: [usedUp(104)] }) },
    'a used up budget': { local: alone, snapshot: snapshot({ account: enterprise, budget: { ...budget, amount_usd: 12_500, local_usd: 13_734.5 }, limits: [{ ...own(), used_pct: 110 }] }) },
  }

  for (const width of [26, 40, 60, 100, 144]) {
    for (const [name, state] of Object.entries(WITH_MONEY)) {
      test(`${name} at ${width} columns cuts no row and splits no value`, () => {
        const lines = screen(input({ width, ...state }))
        expect(overflows(lines, width)).toEqual([])
        const limited = lines.filter(candidate => candidate.key.startsWith('limit') && !candidate.key.startsWith('limits-learning'))
        expect(painted(limited, width).filter(row => row.includes('…'))).toEqual([])
      })
    }
  }
})
