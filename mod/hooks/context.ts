import type { ThemeKey } from 'claude-code'

import type { ConfigGroup, ContextItem, ContextReport, Daemon, History, ItemKind, LiveContext, Local, Part, SourceKind } from '../types'
import { bar, duration, fit, modelName, percent, tokens } from './format'
import type { Line } from './state'
import { CONTEXT_WARNING, MIN_CONTENT, PADDING, WHY_COLUMNS, gap, heading, meter, notice, pair, plain, prose } from './state'

export const LABEL_COLUMNS = 16
export const VALUE_COLUMNS = 7
export const TOP_SHOWN = 6
export const UNUSED_SHOWN = 4
export const TOOL_RESULTS_SHOWN = 3
export const DRILL_SHOWN = 8
export const STALE_BREAKDOWN_MS = 5 * 60_000

const CATEGORY_KINDS: Readonly<Record<string, ItemKind>> = {
  'Memory files': 'memory_file',
  Skills: 'skill',
  'MCP tools': 'mcp_server',
  'Custom agents': 'agent',
}

const PART_KINDS: Readonly<Partial<Record<Part, ItemKind>>> = { memory: 'memory_file', skills: 'skill', mcp: 'mcp_server', agents: 'agent' }

export const THEME_KEYS: readonly ThemeKey[] = [
  'text',
  'inverseText',
  'inactive',
  'subtle',
  'suggestion',
  'remember',
  'success',
  'error',
  'warning',
  'merged',
  'claude',
  'permission',
  'planMode',
  'autoAccept',
  'promptBorder',
  'bashBorder',
  'ide',
  'diffAdded',
  'diffRemoved',
  'diffAddedDimmed',
  'diffRemovedDimmed',
  'diffAddedWord',
  'diffRemovedWord',
]

const PARTS: Record<Part, { label: string; color: ThemeKey }> = {
  system: { label: 'System and tools', color: 'promptBorder' },
  memory: { label: 'Memory files', color: 'autoAccept' },
  skills: { label: 'Skills', color: 'suggestion' },
  mcp: { label: 'MCP', color: 'permission' },
  agents: { label: 'Agents', color: 'remember' },
  files: { label: 'Files', color: 'ide' },
  hooks: { label: 'Hooks', color: 'planMode' },
  reminders: { label: 'Reminders', color: 'subtle' },
  messages: { label: 'Messages', color: 'text' },
}

const SOURCES: [SourceKind, string][] = [
  ['claude_md', 'CLAUDE.md files'],
  ['memory', 'Memory files'],
  ['skill', 'Skills'],
  ['agent', 'Agents'],
  ['output_style', 'Output styles'],
  ['mcp_server', 'MCP servers'],
  ['plugin', 'Plugins'],
]

export type ContextInput = {
  width: number
  nowMs: number
  report: ContextReport | null
  local: Local
  daemon: Daemon
  expanded: readonly string[]
}

type Row = { key: string; name: string; tokens: number; color: ThemeKey; kind?: ItemKind }

type Drill = { items: readonly ContextItem[]; expanded: readonly string[] }

export function themeColor(name: string): ThemeKey {
  return THEME_KEYS.find(key => key === name) ?? 'text'
}

export function reportOf(input: Pick<ContextInput, 'report' | 'local'>): ContextReport | null {
  const { report, local } = input
  return report !== null && (local.session === null || report.session === local.session) ? report : null
}

export function compositionOf(input: Pick<ContextInput, 'report' | 'local'>): LiveContext | null {
  const local = input.local.breakdown
  const stored = reportOf(input)?.live ?? null
  if (local === null) return stored
  return stored !== null && stored.ts_ms > local.ts_ms ? stored : local
}

export function contextScreen(input: ContextInput): Line[] {
  const width = Math.max(MIN_CONTENT, input.width - 2 * PADDING)
  const report = reportOf(input)
  const composition = compositionOf(input)
  const sections = [
    notice(input, width),
    headline(input, composition, width),
    composition !== null ? categories(input, composition, width) : estimate(input, report, width),
    largest(input, report, composition !== null, width),
    unused(report, width),
    toolResults(input, report, width),
    configured(input, report, width),
  ].filter(section => section.length > 0)
  return sections.flatMap((section, index) => (index === 0 ? section : [gap(index), ...section]))
}

function headline(input: ContextInput, composition: LiveContext | null, width: number): Line[] {
  const measure = input.local.measure
  if (composition === null && (measure === null || measure.contextPercent === null)) {
    return [plain('ctx-head', 'Context', { bold: true }), prose('ctx-empty', 'The breakdown appears after your first prompt ✻', { dim: true })]
  }
  const title = composition === null ? 'Context' : `Context · ${modelName(composition.model)}`
  const used = composition?.total_tokens ?? measure?.contextTokens ?? null
  const window = composition?.max_tokens ?? measure?.contextWindow ?? 0
  const fill = composition?.percentage ?? measure?.contextPercent ?? 0
  const color: ThemeKey = fill >= CONTEXT_WARNING ? 'warning' : 'text'
  const lines = [amountLine(title, used, window, width), meter('ctx-bar', '', fill / 100, percent(fill).padStart(4), width, color)]
  const threshold = composition?.auto_compact_threshold ?? null
  if (threshold !== null && used !== null) {
    const due = used >= threshold
    const text = due ? `Auto-compact at ${tokens(threshold)} · due now` : `Auto-compact at ${tokens(threshold)} · ${tokens(threshold - used)} to go`
    lines.push(prose('ctx-compact', text, due ? { color: 'warning' } : { dim: true }))
  }
  return lines
}

function amountLine(title: string, used: number | null, window: number, width: number): Line {
  if (used === null) return pair('ctx-head', title, tokens(window), width, {}, { bold: true })
  const accent = tokens(used)
  const rest = ` / ${tokens(window)}`
  const line = pair('ctx-head', title, `${accent}${rest}`, width, {}, { bold: true })
  line.segments = [line.segments[0]!, { text: accent, color: 'claude', bold: true }, { text: rest }]
  return line
}

function categories(input: ContextInput, composition: LiveContext, width: number): Line[] {
  const rows: Row[] = composition.categories.flatMap((category, index) =>
    category.kind === 'used'
      ? [{ key: `ctx-cat-${index}`, name: category.name, tokens: category.tokens, color: themeColor(category.color), kind: CATEGORY_KINDS[category.name] }]
      : [],
  )
  const drill = { items: reportOf(input)?.items ?? [], expanded: input.expanded }
  const age = Math.max(0, input.nowMs - composition.ts_ms)
  const aged = `By category · ${duration(age)} ago`
  const title = age > STALE_BREAKDOWN_MS && aged.length <= width - WHY_COLUMNS ? aged : 'By category'
  const lines = [heading('ctx-categories', title, width, 'categories'), ...shareRows(rows, width, false, drill)]
  composition.categories.forEach((category, index) => {
    if (category.kind === 'free' || category.kind === 'buffer') {
      lines.push(pair(`ctx-cat-${index}`, `  ${category.name}`, tokens(category.tokens), width, { dim: true }, { dim: true }))
    }
  })
  const deferred = composition.categories.filter(category => category.kind === 'deferred').reduce((sum, category) => sum + category.tokens, 0)
  if (deferred > 0) lines.push(prose('ctx-deferred', `+ ${tokens(deferred)} of tool schemas load on demand`, { dim: true }))
  lines.push(...injected(reportOf(input)?.history ?? null))
  if (input.expanded.includes('categories')) {
    const ago = duration(Math.max(0, input.nowMs - composition.ts_ms))
    lines.push(prose('explain-categories', `Claude Code's own /context breakdown, counted ${ago} ago after a turn. Bars share what is in use; free space and the compaction buffer are the window left. A row with ▸ opens its items.`, { dim: true }))
  }
  return lines
}

function estimate(input: ContextInput, report: ContextReport | null, width: number): Line[] {
  const history = report?.history ?? null
  if (history === null || report === null) return []
  const rows: Row[] = history.now.map(share => ({
    key: `ctx-part-${share.part}`,
    name: PARTS[share.part].label,
    tokens: share.tokens,
    color: PARTS[share.part].color,
    kind: PART_KINDS[share.part],
  }))
  const drill = { items: report.items, expanded: input.expanded }
  const lines = [heading('ctx-estimate', 'Estimated from the transcript', width, 'estimate'), ...shareRows(rows, width, true, drill), ...injected(history)]
  if (input.expanded.includes('estimate')) {
    const ratio = report.ratios.prose.toFixed(1)
    const how = report.ratios.calibrated
      ? `calibrated on ${report.ratios.pairs} memory files Claude Code counted`
      : 'estimated until Claude Code counts your memory files'
    lines.push(
      prose(
        'explain-estimate',
        `Text Claude Code injected, at ${ratio} characters per token (${how}), fitted to the ${tokens(history.measured_tokens)} tokens of your first request. The rest of each request is messages.`,
        { dim: true },
      ),
    )
  }
  return lines
}

function injected(history: History | null): Line[] {
  if (history === null || (history.injected_later_tokens === 0 && history.compactions === 0)) return []
  const parts = [`~${tokens(history.injected_later_tokens)} injected since your first request`]
  if (history.compactions > 0) parts.push(`${history.compactions} compaction${history.compactions === 1 ? '' : 's'}`)
  return [prose('ctx-injected', parts.join(' · '), { dim: true })]
}

function largest(input: ContextInput, report: ContextReport | null, exact: boolean, width: number): Line[] {
  const items = report?.items.slice(0, TOP_SHOWN) ?? []
  if (items.length === 0) return []
  const lines = [heading('ctx-top', 'Largest items', width, 'items'), ...items.map((item, index) => itemRow(`ctx-top-${index}`, item, width))]
  if (input.expanded.includes('items')) {
    const how = exact ? 'counted by Claude Code' : 'estimated from their size on disk'
    lines.push(prose('explain-items', `What your setup loads into every request, ${how}. Skills and agents cost their listing line until used.`, { dim: true }))
  }
  return lines
}

function unused(report: ContextReport | null, width: number): Line[] {
  const items = report?.dead_weight ?? []
  if (items.length === 0) return []
  const total = items.reduce((sum, item) => sum + item.tokens, 0)
  const estimated = items.some(item => item.estimated)
  const lines = [
    pair('ctx-unused', 'Not used in 14 days', `${estimated ? '~' : ''}${tokens(total)} a turn`, width, { dim: true }, { bold: true }),
    ...items.slice(0, UNUSED_SHOWN).map((item, index) => itemRow(`ctx-unused-${index}`, item, width)),
  ]
  const more = moreRow('ctx-unused-more', '  ', items.slice(UNUSED_SHOWN), width)
  return more === null ? lines : [...lines, more]
}

function toolResults(input: ContextInput, report: ContextReport | null, width: number): Line[] {
  const results = report?.tool_results.slice(0, TOOL_RESULTS_SHOWN) ?? []
  if (report === null || results.length === 0) return []
  const lines = [
    heading('ctx-results', 'Largest tool results', width, 'results'),
    ...results.map((result, index) => pair(`ctx-result-${index}`, `  ${toolName(result.tool)}`, `~${tokens(result.tokens)}`, width, {}, { dim: true })),
  ]
  if (input.expanded.includes('results')) {
    const ratio = report.ratios.code.toFixed(1)
    lines.push(
      prose('explain-results', `The largest tool outputs in the conversation since the last compaction, at ${ratio} characters per token. Subagents keep theirs out of it.`, { dim: true }),
    )
  }
  return lines
}

function configured(input: ContextInput, report: ContextReport | null, width: number): Line[] {
  const groups = report?.config.groups ?? []
  if (groups.length === 0) return []
  const lines = [heading('ctx-config', 'Configured', width, 'config')]
  for (const [kind, label] of SOURCES) {
    const mine = groups.filter(group => group.kind === kind)
    if (mine.length === 0) continue
    lines.push(pair(`ctx-config-${kind}`, `  ${label}`, configValue(mine), width, {}, { dim: true }))
  }
  if (input.expanded.includes('config') && report !== null) {
    const ago = duration(Math.max(0, input.nowMs - report.config.scanned_at_ms))
    lines.push(
      prose(
        'explain-config',
        `Read from disk ${ago} ago, never changed. On-demand files load when Claude works in their folder; an MCP server's size is known once a session has loaded it.`,
        { dim: true },
      ),
    )
  }
  return lines
}

function configValue(groups: ConfigGroup[]): string {
  const count = (loading: string) => groups.filter(group => group.loading === loading).reduce((sum, group) => sum + group.count, 0)
  const parts = [String(groups.reduce((sum, group) => sum + group.count, 0))]
  if (count('on_demand') > 0) parts.push(`${count('on_demand')} on demand`)
  if (count('off') > 0) parts.push(`${count('off')} off`)
  return parts.join(' · ')
}

function shareRows(rows: Row[], width: number, estimated: boolean, drill: Drill): Line[] {
  const total = rows.reduce((sum, row) => sum + row.tokens, 0)
  const labelWidth = Math.min(LABEL_COLUMNS, Math.max(...rows.map(row => row.name.length), 1))
  return rows.flatMap(row => {
    const members = row.kind === undefined ? [] : drill.items.filter(item => item.kind === row.kind)
    const toggle = members.length > 0 ? `open-${row.kind}` : undefined
    const isOpen = toggle !== undefined && drill.expanded.includes(toggle)
    const mark = toggle === undefined ? ' ' : isOpen ? '▾' : '▸'
    const label = `${mark} ${fit(row.name, labelWidth).padEnd(labelWidth)} `
    const value = `${estimated ? '~' : ''}${tokens(row.tokens)}`.padStart(VALUE_COLUMNS)
    const cells = bar(total > 0 ? row.tokens / total : 0, width - label.length - value.length - 1)
    const line: Line = {
      key: row.key,
      segments: [{ text: label, dim: true }, { text: cells.filled, color: row.color }, { text: cells.track, dim: true }, { text: ` ${value}` }],
    }
    if (toggle === undefined) return [line]
    line.toggle = toggle
    return isOpen ? [line, ...drillRows(row.key, members, width)] : [line]
  })
}

function drillRows(key: string, items: readonly ContextItem[], width: number): Line[] {
  const lines = items
    .slice(0, DRILL_SHOWN)
    .map((item, index) => pair(`${key}-item-${index}`, `    ${drillLabel(item)}`, `${item.estimated ? '~' : ''}${tokens(item.tokens)}`, width, {}, { dim: true }))
  const more = moreRow(`${key}-more`, '    ', items.slice(DRILL_SHOWN), width)
  return more === null ? lines : [...lines, more]
}

function moreRow(key: string, indent: string, rest: readonly ContextItem[], width: number): Line | null {
  if (rest.length === 0) return null
  const sum = rest.reduce((total, item) => total + item.tokens, 0)
  const value = `${rest.some(item => item.estimated) ? '~' : ''}${tokens(sum)}`
  return pair(key, `${indent}+ ${rest.length} more`, value, width, {}, { dim: true })
}

function drillLabel(item: ContextItem): string {
  return item.detail === null || item.kind === 'skill' ? item.name : `${item.name} · ${item.detail}`
}

function itemRow(key: string, item: ContextItem, width: number): Line {
  return pair(key, `  ${itemLabel(item)}`, `${item.estimated ? '~' : ''}${tokens(item.tokens)}`, width, {}, { dim: true })
}

function itemLabel(item: ContextItem): string {
  if (item.kind === 'memory_file') return item.detail === null ? item.name : `${item.name} · ${item.detail}`
  if (item.kind === 'mcp_server') return item.detail === null ? `MCP ${item.name}` : `MCP ${item.name} · ${item.detail}`
  if (item.kind === 'skill') return `Skill ${item.name}`
  if (item.kind === 'agent') return `Agent ${item.name}`
  return `Style ${item.name}`
}

function toolName(tool: string | null): string {
  if (tool === null) return 'A tool'
  const mcp = /^mcp__(.+?)__(.+)$/.exec(tool)
  if (mcp === null) return tool
  const server = /^plugin_[^_]+_(.+)$/.exec(mcp[1]!)?.[1] ?? mcp[1]
  return `${server} ${mcp[2]}`
}
