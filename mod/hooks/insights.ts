import type { Daemon, Finding, InsightsFailure, InsightsReport, Recap } from '../types'
import { approx, money, percent } from './format'
import type { Line } from './state'
import { MIN_CONTENT, PADDING, WHY_COLUMNS, fitting, gap, notice, pair, plain, prose } from './state'

export const STREAK_SHOWN = 2
export const INDENT = 2
export const RETRY_SECONDS = 60
export const WEEKLY_SHARE = 'of a 5h window a week'
export const SHORT_WEEKLY_SHARE = 'of 5h/wk'

const CONFIDENCE: Readonly<Record<Finding['confidence'], string>> = { high: 'high confidence', medium: 'medium confidence', low: 'low confidence' }

export type InsightsInput = {
  width: number
  report: InsightsReport | null
  failure: InsightsFailure | null
  daemon: Daemon
  expanded: readonly string[]
}

export function weekly(usd: number): string {
  return `${approx(usd)}/week`
}

export function capitalized(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

export function recapText(recap: Recap): string {
  const parts = [`${recap.day}: ${money(recap.cost_usd)}`]
  if (recap.hit_ratio !== null) parts.push(`${percent(recap.hit_ratio * 100)} cache hits`)
  if (recap.win !== null) parts.push(`biggest win: ${recap.win}`)
  return parts.join(' · ')
}

export function insightsScreen(input: InsightsInput): Line[] {
  const width = Math.max(MIN_CONTENT, input.width - 2 * PADDING)
  const report = input.report
  const sections = [
    notice(input, width),
    headline(input, report, width),
    ...(report === null || report.requests === 0 ? [] : [wins(report, width), ...report.findings.map(found => findingLines(input, found))]),
  ].filter(section => section.length > 0)
  return sections.flatMap((section, index) => (index === 0 ? section : [gap(index), ...section]))
}

function headline(input: InsightsInput, report: InsightsReport | null, width: number): Line[] {
  const title = plain('ins-head', 'Insights', { bold: true })
  if (report === null) {
    if (input.daemon.status !== 'ok') return []
    if (input.failure !== null) return [title, ...failed(input.failure)]
    return [title, prose('ins-loading', 'Looking for ways to save… ✻', { dim: true })]
  }
  if (report.requests === 0) return [title, prose('ins-empty', 'Insights need a few sessions of history ✻', { dim: true })]
  if (report.findings.length === 0) return [title, prose('ins-lean', 'No savings to suggest right now ✻ your setup looks lean', { dim: true })]
  const priced = report.findings.filter(found => found.saving_usd !== null)
  const total = priced.reduce((sum, found) => sum + (found.saving_usd ?? 0), 0)
  const lines: Line[] = []
  if (priced.length === 0) {
    lines.push(title)
  } else {
    lines.push({ ...pair('ins-head', 'Insights', `up to ${weekly(total)}`, width - WHY_COLUMNS, { color: 'claude', bold: true }, { bold: true }), why: 'ins-total' })
  }
  const count = report.findings.length
  const tips = `${count} ${count === 1 ? 'tip' : 'tips'} from your last 14 days`
  const shares = priced.map(found => found.saving_pct_5h).filter((pct): pct is number => pct !== null)
  const share = percent(shares.reduce((sum, pct) => sum + pct, 0))
  const summary = shares.length > 0 ? fitting(width, `${tips} · ~${share} ${WEEKLY_SHARE}`, `${tips} · ~${share} ${SHORT_WEEKLY_SHARE}`, tips) : tips
  lines.push(plain('ins-sub', summary, { dim: true }))
  if (input.failure !== null) lines.push(prose('ins-stale', `Could not refresh: ${input.failure.reason}`, { dim: true }))
  if (input.expanded.includes('ins-total')) {
    lines.push(
      prose('explain-ins-total', 'Each saving is a weekly estimate at API prices from your last 14 days. Tips can overlap, so the total is the most you could save.', {
        dim: true,
      }),
    )
  }
  return lines
}

function failed(failure: InsightsFailure): Line[] {
  if (failure.kind === 'missing') {
    return [
      prose('ins-failed', 'This claude-plus daemon is older than the pane and has no Insights', { dim: true }),
      prose('ins-retry', 'Run claude-plus daemon stop, then /plus', { dim: true }),
    ]
  }
  return [
    prose('ins-failed', `Could not look for savings: ${failure.reason}`, { dim: true }),
    prose('ins-retry', `Trying again every ${RETRY_SECONDS} s`, { dim: true }),
  ]
}

function wins(report: InsightsReport, width: number): Line[] {
  const lines: Line[] = []
  const saved = report.wins.saved_usd_7d
  if (saved >= 0.01) {
    const text = `✻ Cache hits saved you ${money(saved)}`
    lines.push(plain('ins-win-saved', fitting(width, `${text} this week`, text), { color: 'success' }))
  }
  const streak = report.wins.hit_streak_days
  if (streak >= STREAK_SHOWN) {
    lines.push(plain('ins-win-streak', fitting(width, `✻ ${streak} days in a row above 90% cache hits`, `✻ ${streak} days above 90% cache hits`), { color: 'success' }))
  }
  return lines
}

function findingLines(input: InsightsInput, found: Finding): Line[] {
  const key = `ins-${found.rule}`
  const title: Line =
    found.saving_usd === null
      ? { key, segments: [{ text: capitalized(found.tip) }], wrap: true }
      : { key, segments: [{ text: `Save ${weekly(found.saving_usd)}`, color: 'success', bold: true }, { text: `: ${found.tip}` }], wrap: true }
  const lines: Line[] = [title, ...found.evidence.map((text, line) => prose(`${key}-${line}`, text, { dim: true }, INDENT))]
  const confidence = CONFIDENCE[found.confidence]
  const room = input.width - 2 * PADDING - WHY_COLUMNS - INDENT
  const share = found.saving_pct_5h === null ? null : `~${percent(found.saving_pct_5h)}`
  const meta = share === null ? confidence : fitting(room, `${share} ${WEEKLY_SHARE} · ${confidence}`, `${share} ${SHORT_WEEKLY_SHARE} · ${confidence}`)
  lines.push({ key: `${key}-meta`, segments: [{ text: `${' '.repeat(INDENT)}${meta}`, dim: true }], why: key })
  if (input.expanded.includes(key)) lines.push(prose(`explain-${key}`, found.how, { dim: true }, INDENT))
  return lines
}
