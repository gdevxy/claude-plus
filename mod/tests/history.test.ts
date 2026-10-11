import { describe, expect, test } from 'claude-code/testing'

import type { HistoryInput } from '../hooks/history'
import { accountChoices, accountLabel, amount, hitFloor, historyScreen, metricChoices, metricsOf, nextAccount, nextMetric, nextRange, rangeChoices, resolvedAccount, shownMetric } from '../hooks/history'
import type { Line } from '../hooks/state'
import type { HistoryReport } from '../types'
import { DAY_MS, OCT_5_MS, OPUS, bucket, contextReport, dayReport, historyReport, local, snapshot, spend } from './fixtures'
import { colorOf, paletteOf, seriesOf } from '../hooks/palette'
import { overflows, painted, trimmed } from './paint'
import { VISIONS, distance } from './vision'

const OK = { status: 'ok', detail: null } as const
const MIN_SERIES_DISTANCE: Readonly<Record<string, number>> = { dark: 15, light: 15, 'dark-daltonized': 15, 'light-daltonized': 12 }

function input(overrides: Partial<HistoryInput> = {}): HistoryInput {
  return { width: 40, range: '7d', metric: 'cost', reports: { '7d': historyReport(), '24h': dayReport() }, failures: {}, context: contextReport(), local: local(), snapshot: null, accounts: [], daemon: OK, expanded: [], theme: 'dark', ...overrides }
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

  test('the controls list every choice and mark the one shown', () => {
    const active = (choices: { text: string; isActive: boolean }[]) => choices.filter(choice => choice.isActive).map(choice => choice.text)
    expect(rangeChoices('30d').map(choice => choice.text)).toEqual(['24h', '7d', '30d', '90d'])
    expect(active(rangeChoices('30d'))).toEqual(['30d'])
    expect(metricChoices('tokens', historyReport()).map(choice => choice.text)).toEqual(['Cost', 'Tokens', '% of 5h'])
    expect(active(metricChoices('tokens', historyReport()))).toEqual(['Tokens'])
    const learning = historyReport({ calibration: { calibrated: false, pairs: 4, needed: 20 } })
    expect(metricChoices('pct', learning)).toEqual([{ text: 'Cost', isActive: true }, { text: 'Tokens', isActive: false }])
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
      '   Plan the Histor… · claude-plus $6.40',
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

  test('draws the day at 40 columns with an hour per bar and a drawn hit chart', () => {
    const lines = historyScreen(input({ range: '24h' }))
    expect(trimmed(painted(lines, 40)).slice(0, 26)).toEqual([
      ' Last 24 hours                   $15.97',
      ' 9 sessions · 160 requests',
      '',
      ' Cost by hour                peak $0.90',
      '                     █      █',
      '                     █     ██     █',
      '                   █ █    ███    ██',
      '                  ██ █    ███   ███',
      '                 ███ █  █ ███  ████   █',
      '                ████ █ ██ ███  ████  ██',
      '               12:00                now',
      '   █ Opus 5.5                    $13.60',
      '   █ Sonnet 5.5                   $2.30',
      '   █ Haiku 4.5                    $0.07',
      '',
      ' Cache hit ratio                  91% ?',
      '                 ▂▄  ▂    ▄      ▂▄  ▂▄',
      '                ▇██▄ █ ▄▇ █▄▇  ▄▇██  ██',
      '   Saved by cache hits           $21.40',
      '   Rebuilds                  2 · ~$0.61',
      '',
      ' 5h limit, highest                  85%',
      '',
      '',
      '                ▁▁▁▁ ▂ ▁▁ ▁▂▂  ▁▁▁▂  ▁▁',
      '',
    ])
    expect(new Set(line(lines, 'hist-hit-chart').chart?.cells.flat().map(cell => cell.fg))).toEqual(new Set([null, 'success']))
  })

  test('the stacked bars are colored by model rank and the scale has no color of its own', () => {
    const lines = historyScreen(input())
    const chart = line(lines, 'hist-spend-chart').chart
    const colors = new Set(chart?.cells.flat().flatMap(cell => [cell.fg, cell.bg]))
    expect([...colors].sort()).toEqual(['claude', 'permission', null].sort())
    expect(line(lines, 'hist-model-0').segments[1]).toEqual({ text: '█', color: 'claude' })
    expect(line(lines, 'hist-model-1').segments[1]).toEqual({ text: '█', color: 'permission' })
    expect(line(lines, 'hist-model-2').segments[1]).toEqual({ text: '█', color: 'ide' })
    expect(line(lines, 'hist-head').segments[1]).toEqual({ text: '$15.97', color: 'claude', bold: true })
    const peak = line(lines, 'hist-peak-chart').chart?.cells.flat().map(cell => cell.fg)
    expect(new Set(peak)).toEqual(new Set([null, 'permission', 'warning']))
  })

  test('every model color and Other stay apart under deuteranopia and protanopia in each color theme', () => {
    const close = Object.keys(MIN_SERIES_DISTANCE).flatMap(theme => {
      const { models, other } = seriesOf(theme)
      const series = [...models, other]
      expect(new Set(series).size).toBe(6)
      const palette = paletteOf(theme)
      if (palette === null) throw new Error(`no ${theme} palette`)
      return Object.entries(VISIONS).flatMap(([vision, matrix]) =>
        series.flatMap((first, index) =>
          series.slice(index + 1).flatMap(second => {
            const apart = distance(colorOf(palette, first), colorOf(palette, second), matrix)
            return apart < (MIN_SERIES_DISTANCE[theme] ?? 0) ? [`${theme} ${vision} ${first}/${second} ${apart.toFixed(1)}`] : []
          }),
        ),
      )
    })
    expect(close).toEqual([])
  })

  test('each color theme ranks its models by its own series', () => {
    const light = historyScreen(input({ theme: 'light' }))
    expect(line(light, 'hist-model-1').segments[1]).toEqual({ text: '█', color: 'planMode' })
    expect(line(light, 'hist-model-2').segments[1]).toEqual({ text: '█', color: 'ide' })
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
    expect(line(lines, 'hist-model-5').segments[1]).toEqual({ text: '█', color: 'subtle' })
    expect(line(lines, 'hist-spend-chart').chart?.cells[0]?.some(cell => cell.bg === 'subtle' || cell.fg === 'subtle')).toBe(true)
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
    expect(text(line(share, 'hist-sessions-0'))).toBe('  Plan the History… · claude-plus ~13%')
    const learning = historyReport({ calibration: { calibrated: false, pairs: 4, needed: 20 } })
    expect(text(line(historyScreen(input({ metric: 'pct', reports: { '7d': learning } })), 'hist-head'))).toBe('Last 7 days                     $15.97')
  })

  test('session and request counts are grouped by thousands as stats prints them', () => {
    const busy = historyReport({ totals: { ...historyReport().totals, sessions: 1234, requests: 14_577, unpriced: 1500 } })
    const lines = historyScreen(input({ reports: { '7d': busy } }))
    expect(text(line(lines, 'hist-sub'))).toBe('1,234 sessions · 14,577 requests')
    expect(text(line(lines, 'hist-unpriced'))).toBe('1,500 unpriced')
  })

  test('a long session title gives way to its project, and both give way to the title when the project cannot fit', () => {
    const report = historyReport({
      sessions: [
        { id: 'a1b2c3d4e5', title: 'Manual dependency updates for the whole monorepo', project: 'claude-plus', last_ms: OCT_5_MS, ...spend(6.4) },
        { id: 'b1b2c3d4e5', title: 'Short', project: 'a-project-name-far-too-long-to-show-beside-a-title', last_ms: OCT_5_MS, ...spend(3.9) },
      ],
    })
    const lines = historyScreen(input({ reports: { '7d': report } }))
    expect(text(line(lines, 'hist-sessions-0'))).toBe('  Manual dependen… · claude-plus $6.40')
    expect(text(line(lines, 'hist-sessions-1'))).toBe('  Short · a-project-name-far-to… $3.90')
    const narrow = historyScreen(input({ width: 26, reports: { '7d': report } }))
    expect(text(line(narrow, 'hist-sessions-0'))).toBe('  Manual dependen… $6.40')
    const projects = historyReport({ projects: [{ name: 'a-very-long-project-name-indeed', path: '/work/a-very-long-project-name-indeed', sessions: 12, ...spend(11.2) }] })
    expect(text(line(historyScreen(input({ reports: { '7d': projects } })), 'hist-projects-0'))).toBe('  a-very-long-pr… · 12 sessions $11.20')
  })

  test('fixed labels fall back to shorter whole words at the narrowest width instead of being cut', () => {
    const rows = trimmed(painted(historyScreen(input({ width: 26 })), 26))
    expect(rows.filter(row => /Saved|\d sessions$|Context|claude-plus|shop|notes/.test(row))).toEqual([
      ' 9 sessions',
      '   Saved           $21.40',
      '   claude-plus     $11.20',
      '   shop             $4.10',
      '   notes            $0.67',
      '   Untitled… · shop $3.90',
      ' Context       46k / 200k',
    ])
    expect(rows.filter(row => /…(?!\s*✻?\s*$)/.test(row) && !/Plan the|Untitled/.test(row))).toEqual([])
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

  test('a report that failed to load says why and that it will try again, instead of loading forever', () => {
    const failed = historyScreen(input({ range: '30d', context: null, failures: { '30d': { kind: 'failed', reason: 'no answer within 3000 ms' } } }))
    expect(failed.map(text)).toEqual(['Last 30 days', 'Could not load the last 30 days: no answer within 3000 ms', 'Trying again every 30 s'])
    expect(failed.slice(1).map(found => found.segments[0]?.dim)).toEqual([true, true])
    const older = historyScreen(input({ range: '30d', context: null, failures: { '30d': { kind: 'missing', reason: 'this daemon has no History' } } }))
    expect(older.map(text)).toEqual(['Last 30 days', 'This claude-plus daemon is older than the pane and has no History', 'Run claude-plus daemon stop, then /plus'])
    const other = historyScreen(input({ range: '30d', context: null, failures: { '7d': { kind: 'failed', reason: 'boom' } } }))
    expect(other.map(text)).toEqual(['Last 30 days', 'Loading the last 30 days… ✻'])
  })

  test('a kept report whose refresh failed stays, with a note under its counts', () => {
    const lines = historyScreen(input({ failures: { '7d': { kind: 'failed', reason: '500: database is locked' } } }))
    expect(lines.slice(0, 3).map(text)).toEqual(['Last 7 days                     $15.97', '9 sessions · 160 requests', 'Could not refresh: 500: database is locked'])
    expect(line(lines, 'hist-stale').segments[0]?.dim).toBe(true)
    expect(lines.some(found => found.key === 'hist-spend-chart')).toBe(true)
  })

  test('the context curve belongs to this session and needs two requests', () => {
    expect(historyScreen(input({ local: local({ session: 's2' }) })).some(line => line.key === 'hist-growth')).toBe(false)
    const single = contextReport()
    const history = single.history
    if (history === null) throw new Error('no history')
    const one = { ...single, history: { ...history, points: history.points.slice(0, 1) } }
    expect(historyScreen(input({ context: one })).some(line => line.key === 'hist-growth')).toBe(false)
  })

  for (const width of [26, 40, 60, 100]) {
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

describe('accounts', () => {
  const accounts = [{ id: 'oauth:me', label: 'Me · Max', kind: 'oauth' as const }, { id: 'oauth:work', label: 'Acme · Enterprise', kind: 'oauth' as const }]
  const current = snapshot({ account: accounts[1]! })
  const shown = { ...historyReport(), account: 'oauth:work', accounts }

  test('resolves current to the session account and all to null', () => {
    expect(resolvedAccount({ range: '7d', metric: 'cost' }, current)).toBe('oauth:work')
    expect(resolvedAccount({ range: '7d', metric: 'cost', account: 'current' }, current)).toBe('oauth:work')
    expect(resolvedAccount({ range: '7d', metric: 'cost', account: 'all' }, current)).toBe(null)
    expect(resolvedAccount({ range: '7d', metric: 'cost', account: 'oauth:me' }, current)).toBe('oauth:me')
    expect(resolvedAccount({ range: '7d', metric: 'cost' }, snapshot({ account: null }))).toBe(null)
    expect(resolvedAccount({ range: '7d', metric: 'cost' }, null)).toBe(null)
  })

  test('cycles current, the other accounts, then all', () => {
    const chart = (account?: string) => ({ range: '7d' as const, metric: 'cost' as const, account })
    expect(nextAccount(chart(), shown, current)).toBe('oauth:me')
    expect(nextAccount(chart('oauth:me'), shown, current)).toBe('all')
    expect(nextAccount(chart('all'), shown, current)).toBe('current')
    expect(nextAccount(chart('all'), shown, snapshot({ account: null }))).toBe('oauth:me')
  })

  test('offers the control whenever there is more than one thing to show', () => {
    expect(accountChoices({ range: '7d', metric: 'cost' }, { accounts: [] }, snapshot({ account: null }))).toEqual([])
    expect(accountChoices({ range: '7d', metric: 'cost' }, null, snapshot({ account: null }))).toEqual([])
    expect(accountChoices({ range: '7d', metric: 'cost' }, null, current).map(c => c.text)).toEqual(['Acme · Enterprise', 'All'])
    expect(accountChoices({ range: '7d', metric: 'cost' }, shown, current).map(c => c.text)).toEqual(['Acme · Enterprise', 'Me · Max', 'All'])
    expect(accountChoices({ range: '7d', metric: 'cost', account: 'oauth:me' }, shown, current).filter(c => c.isActive).map(c => c.text)).toEqual(['Me · Max'])
    expect(accountLabel({ range: '7d', metric: 'cost', account: 'all' }, shown, current)).toBe('All')
    expect(accountLabel({ range: '7d', metric: 'cost' }, shown, current)).toBe('Acme · Enterprise')
  })

  test('one account with spend from before accounts offers itself and All', () => {
    const report = { ...shown, accounts: [accounts[1]!], unknown: true }
    expect(accountChoices({ range: '7d', metric: 'cost' }, report, current).map(c => c.text)).toEqual(['Acme · Enterprise', 'All'])
    expect(nextAccount({ range: '7d', metric: 'cost' }, report, current)).toBe('all')
    expect(nextAccount({ range: '7d', metric: 'cost', account: 'all' }, report, current)).toBe('current')
  })

  test('a day where the other account spent nothing still offers All', () => {
    const day = { ...dayReport(), account: 'oauth:work', accounts: [accounts[1]!] }
    expect(accountChoices({ range: '24h', metric: 'cost' }, day, current).map(c => c.text)).toEqual(['Acme · Enterprise', 'All'])
  })

  test('says that spend without an account shows under All, only when one account is shown', () => {
    const hinted = (report: HistoryReport, width = 60) => historyScreen(input({ width, snapshot: current, accounts: [accounts[1]!], reports: { '7d': report } }))
    const hint = (lines: Line[]) => lines.filter(found => found.key === 'hist-unknown').map(text)
    expect(hint(hinted({ ...shown, unknown: true }))).toEqual(['Spend with no account shows under All'])
    expect(hint(hinted({ ...shown, unknown: false }))).toEqual([])
    expect(hint(hinted({ ...shown, account: null, unknown: true }))).toEqual([])
    const empty = { ...shown, unknown: true, totals: { ...shown.totals, requests: 0, reconciled_usd: 0 } }
    expect(hint(hinted(empty)).length).toBe(1)
    for (const width of [26, 144]) expect(overflows(hinted({ ...shown, unknown: true }, width), width)).toEqual([])
  })

  test('the title names the account shown when there is a choice, and drops it before cutting a word', () => {
    const titled = (width: number, account?: string) => text(line(historyScreen(input({ width, account, snapshot: current, accounts, reports: { '7d': shown } })), 'hist-head'))
    expect(titled(60)).toBe(`Last 7 days · Acme · Enterprise${' '.repeat(21)}$15.97`)
    expect(titled(60, 'all')).toBe(`Last 7 days · All${' '.repeat(35)}$15.97`)
    expect(titled(26)).toBe('Last 7 days       $15.97')
    expect(text(line(historyScreen(input({ width: 60, snapshot: snapshot({ account: null }) })), 'hist-head'))).toBe(`Last 7 days${' '.repeat(41)}$15.97`)
    const loading = (width: number) => text(line(historyScreen(input({ width, snapshot: current, accounts, reports: {} })), 'hist-head'))
    expect(loading(60)).toBe('Last 7 days · Acme · Enterprise')
    expect(loading(26)).toBe('Last 7 days')
  })

  for (const width of [26, 60, 144]) {
    test(`an account title at ${width} columns cuts no row`, () => {
      for (const account of [undefined, 'oauth:me', 'all']) {
        const lines = historyScreen(input({ width, account, snapshot: current, accounts, reports: { '7d': { ...shown, account: account === 'all' ? null : (account ?? 'oauth:work') } } }))
        expect(overflows(lines, width)).toEqual([])
        expect(painted(lines.filter(found => found.key === 'hist-head'), width).filter(row => row.includes('…'))).toEqual([])
      }
    })
  }
})
