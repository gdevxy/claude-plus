import type { ThemeKey } from 'claude-code'

import type { Bucket, ContextReport, Daemon, HistoryRange, HistoryReport, Local, Metric, Reports, Spend } from '../types'
import type { Layout, Piece } from './chart'
import { area, bars, grouped, layout, markers, MARKER, stacked } from './chart'
import { compositionOf, reportOf } from './context'
import { FULL_BLOCK, count, money, modelName, percent, spread, tokens } from './format'
import type { Line } from './state'
import { LIMIT_WARNING, MIN_CONTENT, PADDING, WHY_COLUMNS, fitting, gap, notice, pair, plain, prose } from './state'

export const RANGES: readonly HistoryRange[] = ['24h', '7d', '30d', '90d']
export const METRICS: readonly Metric[] = ['cost', 'tokens', 'pct']
export const SPEND_ROWS = 6
export const HIT_ROWS = 2
export const PEAK_ROWS = 3
export const GROWTH_ROWS = 3
export const TOP_SHOWN = 5
export const HIT_FLOOR_CAP = 0.9
export const MODEL_COLORS: readonly ThemeKey[] = ['claude', 'permission', 'planMode', 'autoAccept', 'bashBorder']
export const OTHER_COLOR: ThemeKey = 'inactive'
export const INDENT = '  '

const RANGE_LABELS: Readonly<Record<HistoryRange, string>> = { '24h': '24 hours', '7d': '7 days', '30d': '30 days', '90d': '90 days' }
const METRIC_LABELS: Readonly<Record<Metric, string>> = { cost: 'Cost', tokens: 'Tokens', pct: '% of 5h' }

export type HistoryInput = {
  width: number
  range: HistoryRange
  metric: Metric
  reports: Reports
  context: ContextReport | null
  local: Local
  daemon: Daemon
  expanded: readonly string[]
}

export function rangeLabel(range: HistoryRange): string {
  return RANGE_LABELS[range]
}

export function metricLabel(metric: Metric): string {
  return METRIC_LABELS[metric]
}

export function nextRange(range: HistoryRange): HistoryRange {
  return RANGES[(RANGES.indexOf(range) + 1) % RANGES.length] ?? '7d'
}

export function metricsOf(report: HistoryReport | null): Metric[] {
  return report?.calibration.calibrated === true ? [...METRICS] : METRICS.filter(metric => metric !== 'pct')
}

export function shownMetric(metric: Metric, report: HistoryReport | null): Metric {
  return metricsOf(report).includes(metric) ? metric : 'cost'
}

export function nextMetric(metric: Metric, report: HistoryReport | null): Metric {
  const allowed = metricsOf(report)
  return allowed[(allowed.indexOf(shownMetric(metric, report)) + 1) % allowed.length] ?? 'cost'
}

export function valueOf(spend: Spend, metric: Metric): number {
  if (metric === 'tokens') return spend.tokens
  if (metric === 'pct') return spend.pct_5h ?? 0
  return spend.cost_usd
}

export function amount(value: number, metric: Metric): string {
  if (metric === 'tokens') return tokens(value)
  if (metric === 'pct') return `~${percent(value)}`
  return money(value)
}

export function historyScreen(input: HistoryInput): Line[] {
  const width = Math.max(MIN_CONTENT, input.width - 2 * PADDING)
  const report = input.reports[input.range] ?? null
  const metric = shownMetric(input.metric, report)
  const busy = report !== null && (report.totals.requests > 0 || report.totals.reconciled_usd > 0)
  const sections = [
    notice(input, width),
    headline(input, report, metric, width),
    ...(report !== null && busy
      ? [spendChart(report, metric, width), cacheChart(input, report, width), peakChart(report, width), top('projects', report, metric, width), top('sessions', report, metric, width)]
      : []),
    growth(input, width),
  ].filter(section => section.length > 0)
  return sections.flatMap((section, index) => (index === 0 ? section : [gap(index), ...section]))
}

function headline(input: HistoryInput, report: HistoryReport | null, metric: Metric, width: number): Line[] {
  const title = `Last ${rangeLabel(input.range)}`
  if (report === null) {
    if (input.daemon.status !== 'ok') return []
    return [plain('hist-head', title, { bold: true }), prose('hist-loading', `Loading the last ${rangeLabel(input.range)}… ✻`, { dim: true })]
  }
  const { totals } = report
  if (totals.requests === 0 && totals.reconciled_usd === 0) {
    return [plain('hist-head', title, { bold: true }), prose('hist-empty', `No requests in the last ${rangeLabel(input.range)} yet ✻`, { dim: true })]
  }
  const value = amount(valueOf(totals, metric), metric)
  const why = metric === 'pct' ? 'hist-pct' : undefined
  const head = pair('hist-head', title, value, why === undefined ? width : width - WHY_COLUMNS, { color: 'claude', bold: true }, { bold: true })
  const lines: Line[] = [why === undefined ? head : { ...head, why }]
  const sessions = `${count(totals.sessions)} ${totals.sessions === 1 ? 'session' : 'sessions'}`
  const requests = `${count(totals.requests)} ${totals.requests === 1 ? 'request' : 'requests'}`
  lines.push(plain('hist-sub', `${sessions} · ${requests}`, { dim: true }))
  if (totals.unpriced > 0) {
    const noun = totals.unpriced === 1 ? 'request on an unknown model is' : 'requests on unknown models are'
    lines.push(prose('hist-unpriced', fitting(width, `${count(totals.unpriced)} ${noun} not priced`, `${count(totals.unpriced)} unpriced`), { dim: true }))
  }
  if (why !== undefined && input.expanded.includes(why)) {
    const { pairs } = report.calibration
    lines.push(
      prose('explain-hist-pct', `Estimated from how far each dollar moved your 5h limit, learned from ${pairs} limit readings. 100% is one full 5h window.`, { dim: true }),
    )
  }
  return lines
}

function per(report: HistoryReport, shape: Layout): string {
  const unit = report.bucket === 'hour' ? 'hour' : 'day'
  return shape.per === 1 ? `by ${unit}` : `per ${shape.per} ${unit}s`
}

function axis(key: string, report: HistoryReport, shape: Layout, groups: Bucket[][], width: number): Line {
  const first = groups[0]?.[0]?.label ?? ''
  const last = report.bucket === 'hour' ? 'now' : 'today'
  return pair(key, `${' '.repeat(shape.offset + shape.gap)}${first}`, last, width, { dim: true }, { dim: true })
}

function spendChart(report: HistoryReport, metric: Metric, width: number): Line[] {
  const shape = layout(report.buckets.length, width)
  const groups = grouped(report.buckets, shape)
  const named = report.models.map(model => model.model)
  const stacks = groups.map(group => {
    const pieces: Piece[] = named.map((model, index) => ({
      value: group.reduce((sum, bucket) => sum + bucket.models.filter(spent => spent.model === model).reduce((s, spent) => s + valueOf(spent, metric), 0), 0),
      color: MODEL_COLORS[index] ?? OTHER_COLOR,
    }))
    const all = group.reduce((sum, bucket) => sum + valueOf(bucket, metric), 0)
    const other = all - pieces.reduce((sum, piece) => sum + piece.value, 0)
    return other > 1e-9 ? [...pieces, { value: other, color: OTHER_COLOR }] : pieces
  })
  const totals = stacks.map(pieces => pieces.reduce((sum, piece) => sum + piece.value, 0))
  const max = Math.max(0, ...totals)
  const lines: Line[] = [
    pair('hist-spend', `${metricLabel(metric)} ${per(report, shape)}`, `peak ${amount(max, metric)}`, width, { dim: true }, { bold: true }),
    { key: 'hist-spend-chart', segments: [], chart: bars(stacks, shape, SPEND_ROWS, max) },
    axis('hist-spend-axis', report, shape, groups, width),
  ]
  const shown = report.models.map((model, index) => ({ name: modelName(model.model), value: valueOf(model, metric), color: MODEL_COLORS[index] ?? OTHER_COLOR }))
  const rest = valueOf(report.totals, metric) - shown.reduce((sum, model) => sum + model.value, 0)
  if (rest > 1e-9) shown.push({ name: 'Other', value: rest, color: OTHER_COLOR })
  shown.forEach((model, index) => lines.push(swatch(`hist-model-${index}`, model.name, amount(model.value, metric), model.color, width)))
  return lines
}

function swatch(key: string, name: string, value: string, color: ThemeKey, width: number): Line {
  const text = spread(`${INDENT}${FULL_BLOCK} ${name}`, value, width)
  const start = INDENT.length + FULL_BLOCK.length
  return {
    key,
    segments: [
      { text: INDENT },
      { text: FULL_BLOCK, color },
      { text: text.slice(start, text.length - value.length), dim: true },
      { text: value },
    ],
  }
}

export function hitFloor(ratios: readonly number[]): number {
  if (ratios.length === 0) return 0
  return Math.min(HIT_FLOOR_CAP, Math.floor(Math.min(...ratios) * 10) / 10)
}

function cacheChart(input: HistoryInput, report: HistoryReport, width: number): Line[] {
  const { totals } = report
  if (totals.hit_ratio === null) return []
  const shape = layout(report.buckets.length, width)
  const groups = grouped(report.buckets, shape)
  const ratios = groups.map(group => {
    const prompt = group.reduce((sum, bucket) => sum + bucket.prompt_tokens, 0)
    return prompt > 0 ? group.reduce((sum, bucket) => sum + bucket.cache_read_tokens, 0) / prompt : null
  })
  const floor = hitFloor(ratios.filter((ratio): ratio is number => ratio !== null))
  const stacks = ratios.map((ratio): Piece[] => (ratio === null ? [] : [{ value: Math.max(0, ratio - floor), color: 'success' }]))
  const head = pair('hist-hit', 'Cache hit ratio', percent(totals.hit_ratio * 100), width - WHY_COLUMNS, {}, { bold: true })
  const lines: Line[] = [{ ...head, why: 'hist-hit' }]
  if (input.expanded.includes('hist-hit')) {
    const scale = floor > 0 ? ` The bars start at ${percent(floor * 100)}, so small changes show.` : ''
    lines.push(prose('explain-hist-hit', `The share of prompt tokens read from the cache instead of sent again.${scale}`, { dim: true }))
  }
  lines.push({ key: 'hist-hit-chart', segments: [], chart: bars(stacks, shape, HIT_ROWS, 1 - floor) })
  if (totals.saved_usd >= 0.01) lines.push(pair('hist-saved', `${INDENT}Saved by cache hits`, money(totals.saved_usd), width, { color: 'success' }, { dim: true }))
  if (totals.rebuilds > 0) {
    lines.push(pair('hist-rebuilds', `${INDENT}Rebuilds`, `${totals.rebuilds} · ~${money(totals.rebuild_usd)}`, width, {}, { dim: true }))
  }
  return lines
}

function peakChart(report: HistoryReport, width: number): Line[] {
  const peak = report.totals.peak_5h_pct
  if (peak === null) return []
  const shape = layout(report.buckets.length, width)
  const groups = grouped(report.buckets, shape)
  const stacks = groups.map((group): Piece[] => {
    const peaks = group.map(bucket => bucket.peak_5h_pct).filter((value): value is number => value !== null)
    if (peaks.length === 0) return []
    const highest = Math.max(...peaks)
    return [{ value: highest, color: highest >= LIMIT_WARNING ? 'warning' : 'permission' }]
  })
  return [
    pair('hist-peak', '5h limit, highest', percent(peak), width, peak >= LIMIT_WARNING ? { color: 'warning' } : {}, { bold: true }),
    { key: 'hist-peak-chart', segments: [], chart: bars(stacks, shape, PEAK_ROWS, 100) },
  ]
}

function top(kind: 'projects' | 'sessions', report: HistoryReport, metric: Metric, width: number): Line[] {
  const rows =
    kind === 'projects'
      ? report.projects.map(project => ({ name: `${project.name} · ${project.sessions} ${project.sessions === 1 ? 'session' : 'sessions'}`, spend: project }))
      : report.sessions.map(session => ({ name: `${session.title ?? `Untitled ${session.id.slice(0, 8)}`} · ${session.project}`, spend: session }))
  if (rows.length === 0) return []
  return [
    plain(`hist-${kind}`, kind === 'projects' ? 'Top projects' : 'Top sessions', { bold: true }),
    ...rows.slice(0, TOP_SHOWN).map((row, index) => pair(`hist-${kind}-${index}`, `${INDENT}${row.name}`, amount(valueOf(row.spend, metric), metric), width, {}, { dim: true })),
  ]
}

function growth(input: HistoryInput, width: number): Line[] {
  const report = reportOf({ report: input.context, local: input.local })
  const points = report?.history?.points ?? []
  if (points.length < 2) return []
  const values = points.map(point => point.prompt_tokens)
  const window = compositionOf({ report: input.context, local: input.local })?.max_tokens ?? input.local.measure?.contextWindow ?? Math.max(...values)
  const last = values[values.length - 1] ?? 0
  const compacted = points.flatMap((point, index) => (point.compacted ? [Math.min(width - 1, Math.floor((index * width) / points.length))] : []))
  const curve = area(values, width, GROWTH_ROWS, window, 'claude')
  const lines: Line[] = [
    pair('hist-growth', 'Context this session', `${tokens(last)} / ${tokens(window)}`, width, {}, { bold: true }),
    { key: 'hist-growth-chart', segments: [], chart: compacted.length > 0 ? stacked(markers(width, compacted, 'warning'), curve) : curve },
  ]
  if (compacted.length > 0) {
    const noun = compacted.length === 1 ? 'compaction' : 'compactions'
    lines.push({ key: 'hist-growth-note', segments: [{ text: INDENT }, { text: MARKER, color: 'warning' }, { text: ` ${compacted.length} ${noun}`, dim: true }] })
  }
  return lines
}
