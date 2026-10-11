import { describe, expect, test } from 'claude-code/testing'

import type { InsightsInput } from '../hooks/insights'
import { capitalized, insightsScreen, recapText, weekly } from '../hooks/insights'
import type { Line } from '../hooks/state'
import { finding, insightsReport } from './fixtures'
import { overflows, painted, trimmed } from './paint'

const OK = { status: 'ok', detail: null } as const

function input(overrides: Partial<InsightsInput> = {}): InsightsInput {
  return { width: 40, report: insightsReport(), failure: null, daemon: OK, expanded: [], ...overrides }
}

function text(line: Line): string {
  return line.segments.map(segment => segment.text).join('')
}

function line(lines: Line[], key: string): Line {
  const found = lines.find(candidate => candidate.key === key)
  if (found === undefined) throw new Error(`no line ${key}`)
  return found
}

describe('insights screen', () => {
  test('draws ranked tips with their payoff first at 40 columns', () => {
    expect(trimmed(painted(insightsScreen(input()), 40))).toEqual([
      ' Insights           up to ~$5.55/week ?',
      ' 3 tips from your last 14 days',
      '',
      ' ✻ Cache hits saved you $21.40',
      ' ✻ 5 days in a row above 90% cache hits',
      '',
      ' Save ~$4.20/week: start fresh after a',
      ' break instead of rewarming a long',
      ' context',
      '   12 cache rebuilds after a break, in',
      '   5 sessions, $8.40',
      '   Typical break 14m, the cache lasts',
      '   5m',
      '   Each rebuild rewrote ~86k tokens',
      '   ~8% of 5h/wk · high confidence     ?',
      '',
      ' Save ~$1.35/week: turn off the linear',
      ' MCP server, unused for 14 days',
      '   linear: ~4.1k tokens per request,',
      '   unused for 30 days',
      '   Carried on 1,180 requests in this',
      '   project over 14 days',
      '   ~3% of 5h/wk · high confidence     ?',
      '',
      ' Break your biggest asks into smaller',
      ' steps',
      '   Migrate the schema: $6.20, 84',
      '   requests',
      '   Your typical task costs $0.31 across',
      '   412 tasks',
      '   low confidence                     ?',
    ])
  })

  test('the payoff is the success color and the headline total is the accent', () => {
    const lines = insightsScreen(input())
    expect(line(lines, 'ins-idle_rebuilds').segments[0]).toEqual({ text: 'Save ~$4.20/week', color: 'success', bold: true })
    expect(line(lines, 'ins-head').segments[1]).toEqual({ text: 'up to ~$5.55/week', color: 'claude', bold: true })
    expect(line(lines, 'ins-win-saved').segments[0]?.color).toBe('success')
    expect(line(lines, 'ins-costly_tasks').segments).toEqual([{ text: 'Break your biggest asks into smaller steps' }])
  })

  test('wider panes keep the 5h share on the summary and the week on the win', () => {
    const lines = insightsScreen(input({ width: 60 }))
    expect(text(line(lines, 'ins-sub'))).toBe('3 tips from your last 14 days · ~11% of a 5h window a week')
    expect(text(line(lines, 'ins-idle_rebuilds-meta'))).toBe('  ~8% of a 5h window a week · high confidence')
    expect(text(line(lines, 'ins-win-saved'))).toBe('✻ Cache hits saved you $21.40 this week')
  })

  test('a question mark explains the total and each tip', () => {
    const lines = insightsScreen(input({ expanded: ['ins-total', 'ins-unused_mcp_servers'] }))
    expect(text(line(lines, 'explain-ins-total'))).toBe(
      'Each saving is a weekly estimate at API prices from your last 14 days. Tips can overlap, so the total is the most you could save.',
    )
    expect(text(line(lines, 'explain-ins-unused_mcp_servers'))).toBe("Token sizes come from Claude Code's own /context breakdown.")
    expect(line(lines, 'explain-ins-unused_mcp_servers').indent).toBe(2)
    expect(lines.some(found => found.key === 'explain-ins-idle_rebuilds')).toBe(false)
  })

  test('loading, no history, a lean setup and a down daemon each read plainly', () => {
    expect(insightsScreen(input({ report: null })).map(text)).toEqual(['Insights', 'Looking for ways to save… ✻'])
    expect(insightsScreen(input({ report: insightsReport({ requests: 0, findings: [] }) })).map(text)).toEqual(['Insights', 'Insights need a few sessions of history ✻'])
    const lean = insightsScreen(input({ report: insightsReport({ findings: [], wins: { saved_usd_7d: 0, hit_streak_days: 1 } }) }))
    expect(lean.map(text)).toEqual(['Insights', 'No savings to suggest right now ✻ your setup looks lean'])
    const down = insightsScreen(input({ report: null, daemon: { status: 'unreachable', detail: null } }))
    expect(down.map(found => found.key)).toEqual(['daemon', 'daemon-fix'])
    const kept = insightsScreen(input({ daemon: { status: 'unreachable', detail: null } }))
    expect(kept.slice(0, 3).map(found => found.key)).toEqual(['daemon', 'daemon-fix', 'gap-1'])
    expect(kept.some(found => found.key === 'ins-idle_rebuilds')).toBe(true)
  })

  test('a report that failed to load says why and that it will try again, instead of loading forever', () => {
    const failed = insightsScreen(input({ report: null, failure: { kind: 'failed', reason: 'no answer within 3000 ms' } }))
    expect(failed.map(text)).toEqual(['Insights', 'Could not look for savings: no answer within 3000 ms', 'Trying again every 60 s'])
    expect(failed.slice(1).map(found => found.segments[0]?.dim)).toEqual([true, true])
    const older = insightsScreen(input({ report: null, failure: { kind: 'missing', reason: 'this daemon has no Insights' } }))
    expect(older.map(text)).toEqual(['Insights', 'This claude-plus daemon is older than the pane and has no Insights', 'Run claude-plus daemon stop, then /plus'])
    const kept = insightsScreen(input({ failure: { kind: 'failed', reason: '500: database is locked' } }))
    expect(line(kept, 'ins-stale').segments.map(segment => [segment.text, segment.dim])).toEqual([['Could not refresh: 500: database is locked', true]])
    expect(kept.indexOf(line(kept, 'ins-stale'))).toBe(kept.indexOf(line(kept, 'ins-sub')) + 1)
    expect(kept.some(found => found.key === 'ins-idle_rebuilds')).toBe(true)
  })

  test('tips without a saving keep a plain heading and no total', () => {
    const report = insightsReport({ findings: [finding({ rule: 'retention', tip: 'keep 90 days of transcripts', saving_usd: null, saving_pct_5h: null })] })
    const lines = insightsScreen(input({ report }))
    expect(line(lines, 'ins-head')).toEqual({ key: 'ins-head', segments: [{ text: 'Insights', bold: true }] })
    expect(text(line(lines, 'ins-sub'))).toBe('1 tip from your last 14 days')
    expect(text(line(lines, 'ins-retention-meta'))).toBe('  high confidence')
  })

  test('the recap reads like a greeting and leaves out what it does not know', () => {
    const recap = insightsReport().recap
    if (recap === null) throw new Error('no recap')
    expect(recapText(recap)).toBe('Yesterday: $12.40 · 94% cache hits · biggest win: 5 days in a row above 90% cache hits')
    expect(recapText({ ...recap, day: 'Oct 8', hit_ratio: null, win: null })).toBe('Oct 8: $12.40')
    expect([weekly(4.2), capitalized('keep it'), capitalized('')]).toEqual(['~$4.20/week', 'Keep it', ''])
  })

  for (const width of [40, 60, 100]) {
    test(`no row is cut or wider than the pane at ${width} columns`, () => {
      for (const report of [insightsReport(), null, insightsReport({ findings: [] })]) {
        const lines = insightsScreen(input({ width, report, expanded: ['ins-total', 'ins-idle_rebuilds', 'ins-unused_mcp_servers', 'ins-costly_tasks'] }))
        expect(overflows(lines, width)).toEqual([])
        expect(painted(lines, width).filter(row => row.length > width)).toEqual([])
      }
    })
  }
})
