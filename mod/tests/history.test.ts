import { describe, expect, test } from 'claude-code/testing'

import type { HistoryInput } from '../hooks/history'
import { amount, hitFloor, historyScreen, metricsOf, nextMetric, nextRange, shownMetric } from '../hooks/history'
import type { Line } from '../hooks/state'
import type { HistoryReport } from '../types'
import { DAY_MS, OCT_5_MS, OPUS, bucket, contextReport, dayReport, historyReport, local, spend } from './fixtures'
import { overflows, painted, trimmed } from './paint'

const OK = { status: 'ok', detail: null } as const

function input(overrides: Partial<HistoryInput> = {}): HistoryInput {
  return { width: 40, range: '7d', metric: 'cost', reports: { '7d': historyReport(), '24h': dayReport() }, context: contextReport(), local: local(), daemon: OK, expanded: [], ...overrides }
}

function line(lines: Line[], key: string): Line {
  const found = lines.find(candidate => candidate.key === key)
  if (found === undefined) throw new Error(`no line ${key}`)
  return found
}

function text(found: Line): string {
  return found.segments.map(segment => segment.text).join('')
}

function quarter(): HistoryReport {
  const buckets = Array.from({ length: 90 }, (_, index) => bucket(OCT_5_MS - (83 - index) * DAY_MS, `day ${index}`, [[OPUS, index % 4]], 0.9, null))
  return historyReport({ range: '90d', buckets })
}

describe('history screen', () => {
  test('ranges cycle from a day to ninety days and round again', () => {
    expect([nextRange('24h'), nextRange('7d'), nextRange('30d'), nextRange('90d')]).toEqual(['7d', '30d', '90d', '24h'])
  })

  test('the metric cycles through what the report can show, skipping the 5h share until it is calibrated', () => {
    const calibrated = historyReport()
    const learning = historyReport({ calibration: { calibrated: false, pairs: 4, needed: 20 } })
    expect([nextMetric('cost', calibrated), nextMetric('tokens', calibrated), nextMetric('pct', calibrated)]).toEqual(['tokens', 'pct', 'cost'])
    expect([nextMetric('cost', learning), nextMetric('tokens', learning), nextMetric('pct', learning)]).toEqual(['tokens', 'cost', 'tokens'])
    expect([metricsOf(null), shownMetric('pct', learning), shownMetric('pct', calibrated)]).toEqual([['cost', 'tokens'], 'cost', 'pct'])
    expect([amount(15.97, 'cost'), amount(1_597_000, 'tokens'), amount(31.94, 'pct')]).toEqual(['$15.97', '1.6M', '~32%'])
  })

  test('draws the week at 40 columns with the charts as cells', () => {
    const lines = historyScreen(input())
    expect(trimmed(painted(lines, 40))).toEqual([
      ' Last 7 days                     $15.97',
      ' 9 sessions · 160 requests',
      '',
      ' Cost by day                 peak $4.45',
      '                         ▅▅▅▅ ▃▃▃▃',
      '                         ▁▁▁▁ ████',
      '               ▇▇▇▇      ████ ████',
      '               ████      ████ ████ ▇▇▇▇',
      '     ▅▅▅▅      ████ ▁▁▁▁ ████ ████ ████',
      '     ████      ████ ████ ████ ████ ████',
      '     Oct 5                        today',
      '   █ Opus 5.5                    $13.60',
      '   █ Sonnet 5.5                   $2.30',
      '   █ Haiku 4.5                    $0.07',
      '',
      ' Cache hit ratio                  91% ?',
      '               ▃▃▃▃      ▄▄▄▄ ▅▅▅▅ ▃▃▃▃',
      '     ▆▆▆▆      ████ ▃▃▃▃ ████ ████ ████',
      '   Saved by cache hits           $21.40',
      '   Rebuilds                  2 · ~$0.61',
      '',
      ' 5h limit, highest                  85%',
      '                         ▄▄▄▄',
      '               ▄▄▄▄      ████ ▇▇▇▇',
      '     ▃▃▃▃      ████ ▅▅▅▅ ████ ████ ▇▇▇▇',
      '',
      ' Top projects',
      '   claude-plus · 5 sessions      $11.20',
      '   shop · 3 sessions              $4.10',
      '   notes · 1 session              $0.67',
      '',
      ' Top sessions',
      '   Plan the History view · claud… $6.40',
      '   Untitled a1b2c3d4 · shop       $3.90',
      '',
      ' Context this session        46k / 200k',
      '                         ┊',
      '                      ⣀⣀⣀⡀',
      '               ⣤⣤⣤⣴⣶⣶⣶⣿⣿⣿⡇',
      ' ⣀⣀⣀⣠⣤⣤⣤⣶⣶⣶⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⣤⣤⣤⣤⣤⣤⣴⣶⣶⣶⣶⣶⣶',
      '   ┊ 1 compaction',
    ])
  })

  test('the stacked bars are colored by model rank and the scale has no color of its own', () => {
    const lines = historyScreen(input())
    const chart = line(lines, 'hist-spend-chart').chart
    const colors = new Set(chart?.cells.flat().flatMap(cell => [cell.fg, cell.bg]))
    expect([...colors].sort()).toEqual(['claude', 'permission', null].sort())
    expect(line(lines, 'hist-model-0').segments[1]).toEqual({ text: '█', color: 'claude' })
    expect(line(lines, 'hist-model-1').segments[1]).toEqual({ text: '█', color: 'permission' })
    expect(line(lines, 'hist-model-2').segments[1]).toEqual({ text: '█', color: 'planMode' })
    expect(line(lines, 'hist-head').segments[1]).toEqual({ text: '$15.97', color: 'claude', bold: true })
    const peak = line(lines, 'hist-peak-chart').chart?.cells.flat().map(cell => cell.fg)
    expect(new Set(peak)).toEqual(new Set([null, 'permission', 'warning']))
  })

  test('models beyond the top five are drawn together as Other', () => {
    const names = ['opus-5-5', 'sonnet-5-5', 'haiku-4-5', 'opus-5-1', 'sonnet-5-1', 'haiku-4-1'].map(name => `claude-${name}`)
    const spent = names.map((name, index): [string, number] => [name, index === 5 ? 10 : 6 - index])
    const report = historyReport({
      buckets: [bucket(OCT_5_MS, 'Oct 5', spent, 0.9, null)],
      models: names.slice(0, 5).map((model, index) => ({ model, ...spend(6 - index) })),
      totals: { ...historyReport().totals, ...spend(30) },
    })
    const lines = historyScreen(input({ reports: { '7d': report } }))
    expect(text(line(lines, 'hist-model-5'))).toBe('  █ Other' + ' '.repeat(23) + '$10.00')
    expect(line(lines, 'hist-model-5').segments[1]).toEqual({ text: '█', color: 'inactive' })
    expect(line(lines, 'hist-spend-chart').chart?.cells[0]?.some(cell => cell.bg === 'inactive' || cell.fg === 'inactive')).toBe(true)
  })

  test('ninety days at 40 columns merge three days into each bar', () => {
    const lines = historyScreen(input({ range: '90d', reports: { '90d': quarter() } }))
    expect(text(line(lines, 'hist-spend'))).toBe('Cost per 3 days             peak $6.00')
    expect(text(line(lines, 'hist-spend-axis'))).toBe('        day 0                    today')
    expect(line(lines, 'hist-spend-chart').chart?.columns).toBe(38)
  })

  test('a day reads by hour and ends now', () => {
    const lines = historyScreen(input({ range: '24h' }))
    expect(text(line(lines, 'hist-head')).startsWith('Last 24 hours')).toBe(true)
    expect(text(line(lines, 'hist-spend')).startsWith('Cost by hour')).toBe(true)
    expect(text(line(lines, 'hist-spend-axis')).trim().split(/\s+/)).toEqual(['12:00', 'now'])
  })

  test('tokens and the 5h share change the numbers but not the ranking', () => {
    const tokens = historyScreen(input({ metric: 'tokens' }))
    expect(text(line(tokens, 'hist-head'))).toBe('Last 7 days                       1.6M')
    expect(text(line(tokens, 'hist-spend')).startsWith('Tokens by day')).toBe(true)
    expect(text(line(tokens, 'hist-projects-0'))).toBe('  claude-plus · 5 sessions        1.1M')
    const share = historyScreen(input({ metric: 'pct', expanded: ['hist-pct'] }))
    expect(text(line(share, 'hist-head'))).toBe('Last 7 days                     ~32%')
    expect(line(share, 'hist-head').why).toBe('hist-pct')
    expect(text(line(share, 'explain-hist-pct'))).toBe('Estimated from how far each dollar moved your 5h limit, learned from 31 limit readings. 100% is one full 5h window.')
    expect(text(line(share, 'hist-sessions-0'))).toBe('  Plan the History view · claude… ~13%')
    const learning = historyReport({ calibration: { calibrated: false, pairs: 4, needed: 20 } })
    expect(text(line(historyScreen(input({ metric: 'pct', reports: { '7d': learning } })), 'hist-head'))).toBe('Last 7 days                     $15.97')
  })

  test('the hit ratio scale starts at the lowest tenth and says so', () => {
    expect([hitFloor([0.76, 0.95]), hitFloor([1, 1]), hitFloor([]), hitFloor([0.05])]).toEqual([0.7, 0.9, 0, 0])
    const lines = historyScreen(input({ expanded: ['hist-hit'] }))
    expect(text(line(lines, 'explain-hist-hit'))).toBe('The share of prompt tokens read from the cache instead of sent again. The bars start at 70%, so small changes show.')
  })

  test('a report not loaded yet, an empty one and a down daemon each read plainly', () => {
    const loading = historyScreen(input({ range: '30d', context: null }))
    expect(loading.map(text)).toEqual(['Last 30 days', 'Loading the last 30 days… ✻'])
    const empty = historyReport({ totals: { ...historyReport().totals, requests: 0, reconciled_usd: 0 } })
    expect(historyScreen(input({ reports: { '7d': empty }, context: null })).map(text)).toEqual(['Last 7 days', 'No requests in the last 7 days yet ✻'])
    const down = historyScreen(input({ range: '30d', context: null, daemon: { status: 'unreachable', detail: null } }))
    expect(down.map(line => line.key)).toEqual(['daemon', 'daemon-fix'])
    const kept = historyScreen(input({ daemon: { status: 'unreachable', detail: null } }))
    expect(kept.slice(0, 3).map(line => line.key)).toEqual(['daemon', 'daemon-fix', 'gap-1'])
    expect(kept.some(line => line.key === 'hist-spend-chart')).toBe(true)
  })

  test('the context curve belongs to this session and needs two requests', () => {
    expect(historyScreen(input({ local: local({ session: 's2' }) })).some(line => line.key === 'hist-growth')).toBe(false)
    const single = contextReport()
    const history = single.history
    if (history === null) throw new Error('no history')
    const one = { ...single, history: { ...history, points: history.points.slice(0, 1) } }
    expect(historyScreen(input({ context: one })).some(line => line.key === 'hist-growth')).toBe(false)
  })

  for (const width of [40, 60, 100]) {
    test(`no row is cut and every chart fills the width at ${width} columns`, () => {
      for (const [range, report] of [['7d', historyReport()], ['24h', dayReport()], ['90d', quarter()]] as const) {
        for (const metric of ['cost', 'tokens', 'pct'] as const) {
          const lines = historyScreen(input({ width, range, metric, reports: { [range]: report }, expanded: ['hist-hit', 'hist-pct'] }))
          expect(overflows(lines, width)).toEqual([])
          expect(lines.filter(found => found.chart !== undefined && found.chart.columns !== width - 2).map(found => found.key)).toEqual([])
          expect(painted(lines, width).filter(row => row.length > width)).toEqual([])
        }
      }
    })
  }
})
