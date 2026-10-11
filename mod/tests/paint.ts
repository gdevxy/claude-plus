import { glyphRows } from '../hooks/chart'
import type { Line } from '../hooks/state'

export const ONE_ROW_KEYS = [/^cache-note$/, /^cache-rewarm$/, /^context-empty$/, /^unpriced$/, /^limits-learning/, /^limit-.*-detail$/, /^daemon-fix$/]

export function textOf(line: Line): string {
  return line.segments.map(segment => segment.text).join('')
}

export function truncated(text: string, width: number): string {
  return text.length <= width ? text : `${text.slice(0, Math.max(0, width - 1))}…`
}

export function wrapped(text: string, width: number): string[] {
  const rows: string[] = []
  let row = ''
  for (const word of text.split(' ')) {
    const joined = row === '' ? word : `${row} ${word}`
    if (joined.length <= width) {
      row = joined
      continue
    }
    if (row !== '') rows.push(row)
    row = word
    while (row.length > width) {
      rows.push(row.slice(0, width))
      row = row.slice(width)
    }
  }
  rows.push(row)
  return rows
}

export function rowsOf(line: Line, inner: number): string[] {
  if (line.chart !== undefined) return glyphRows(line.chart)
  const text = textOf(line)
  if (line.why !== undefined) return [`${truncated(text, inner - 1).padEnd(inner - 1)}?`]
  const indent = ' '.repeat(line.indent ?? 0)
  const room = inner - indent.length
  const rows = line.wrap === true ? wrapped(text, room) : [truncated(text, room)]
  return rows.map(row => `${indent}${row}`)
}

export function painted(lines: Line[], width: number): string[] {
  return lines.flatMap(line => rowsOf(line, width - 2)).map(row => ` ${row}`)
}

export function trimmed(rows: string[]): string[] {
  return rows.map(row => row.trimEnd())
}

export function overflows(lines: Line[], width: number): string[] {
  const inner = width - 2
  const problems: string[] = []
  for (const line of lines) {
    const text = textOf(line)
    const room = inner - (line.indent ?? 0) - (line.why === undefined ? 0 : 1)
    if (line.wrap !== true && text.length > room) problems.push(`${line.key} is cut at ${width}: ${text}`)
    if (ONE_ROW_KEYS.some(pattern => pattern.test(line.key)) && rowsOf(line, inner).length > 1) problems.push(`${line.key} wraps at ${width}: ${text}`)
  }
  return problems
}

type Drawn = { type: string; props?: Record<string, unknown>; children?: unknown[] }

function isDrawn(child: unknown): child is Drawn {
  return typeof child === 'object' && child !== null && 'type' in child
}

function cells(value: unknown): number {
  return typeof value === 'number' ? value : 0
}

function shownText(node: Drawn): string {
  if (node.type === 'Button' && node.props?.label !== undefined) {
    const hotkey = node.props.plain === true && typeof node.props.hotkey === 'string' ? `${node.props.hotkey}: ` : ''
    return `${hotkey}${String(node.props.label)}`
  }
  return (node.children ?? []).map(child => (typeof child === 'string' ? child : isDrawn(child) ? shownText(child) : '')).join('')
}

export function paintTree(node: Drawn, width: number): string[] {
  const props = node.props ?? {}
  const outer = typeof props.width === 'number' ? props.width : width
  if (node.type === 'Text') {
    const text = shownText(node)
    return props.wrap === 'wrap' ? wrapped(text, outer) : [truncated(text, outer)]
  }
  if (node.type === 'Button') return [shownText(node)]
  if (node.type === 'Raster') return rasterRows(String(props.cells), cells(props.columns), cells(props.rows))
  if (node.type !== 'Box') throw new Error(`cannot paint a ${node.type}`)
  const left = cells(props.paddingX) + cells(props.paddingLeft)
  const inner = outer - left - cells(props.paddingX)
  const children = (node.children ?? []).filter(isDrawn)
  const rows =
    props.flexDirection === 'column'
      ? children.flatMap(child => paintTree(child, inner))
      : across(children, inner, props.justifyContent === 'space-between', cells(props.gap))
  return rows.map(row => `${' '.repeat(left)}${row}`)
}

function across(children: Drawn[], width: number, spread: boolean, gap: number): string[] {
  const [first, ...rest] = children
  if (first === undefined) return ['']
  if (!spread && gap > 0) return [children.map(child => paintTree(child, width).join('')).join(' '.repeat(gap))]
  const right = rest.map(child => paintTree(child, width).join('')).join('')
  const rows = paintTree(first, width - right.length)
  const [top = '', ...below] = rows
  return [spread ? `${top.padEnd(width - right.length)}${right}` : `${top}${right}`, ...below]
}

export function decoded(cells: string): number[] {
  const bytes = Uint8Array.from(atob(cells), char => char.charCodeAt(0))
  const view = new DataView(bytes.buffer)
  return Array.from({ length: Math.floor(bytes.length / 4) }, (_, index) => view.getUint32(index * 4, true))
}

export function rasterRows(cells: string, columns: number, rows: number): string[] {
  const words = decoded(cells)
  return Array.from({ length: rows }, (_, row) =>
    Array.from({ length: columns }, (_, column) => String.fromCharCode(words[(row * columns + column) * 3] ?? 32)).join(''),
  )
}
