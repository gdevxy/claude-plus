import type { ThemeKey } from 'claude-code'

import type { Palette } from './palette'
import { colorOf } from './palette'

export const LOWER = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
export const BRAILLE_BASE = 0x2800
export const MARKER = '┊'
const LEFT_DOTS = [0x40, 0x04, 0x02, 0x01]
const RIGHT_DOTS = [0x80, 0x20, 0x10, 0x08]
const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

export type Cell = { glyph: string; fg: ThemeKey | null; bg: ThemeKey | null }

export type Grid = { columns: number; rows: number; cells: Cell[][] }

export type Layout = { per: number; bars: number; cell: number; gap: number; dropped: number; offset: number }

export type Piece = { value: number; color: ThemeKey }

export type ColoredRun = { text: string; fg: ThemeKey | null; bg: ThemeKey | null }

const EMPTY: Cell = { glyph: ' ', fg: null, bg: null }

export function blank(columns: number, rows: number): Grid {
  return { columns, rows, cells: Array.from({ length: rows }, () => Array.from({ length: columns }, () => EMPTY)) }
}

export function layout(buckets: number, width: number): Layout {
  const count = Math.max(1, buckets)
  const per = Math.max(1, Math.ceil(count / width))
  const bars = Math.max(1, Math.floor(count / per))
  const cell = Math.max(1, Math.floor(width / bars))
  return { per, bars, cell, gap: cell >= 3 ? 1 : 0, dropped: count - bars * per, offset: width - bars * cell }
}

export function grouped<T>(items: readonly T[], shape: Layout): T[][] {
  const kept = items.slice(shape.dropped)
  return Array.from({ length: shape.bars }, (_, index) => kept.slice(index * shape.per, (index + 1) * shape.per))
}

function put(grid: Grid, row: number, column: number, cell: Cell): void {
  const line = grid.cells[row]
  if (line !== undefined && column >= 0 && column < grid.columns) line[column] = cell
}

function heaviest(pieces: readonly { color: ThemeKey; eighths: number }[]): ThemeKey | null {
  let best: { color: ThemeKey; eighths: number } | null = null
  for (const piece of pieces) if (piece.eighths > 0 && (best === null || piece.eighths > best.eighths)) best = piece
  return best?.color ?? null
}

export function columnCells(pieces: readonly Piece[], max: number, rows: number): Cell[] {
  const total = pieces.reduce((sum, piece) => sum + Math.max(0, piece.value), 0)
  const height = total <= 0 || max <= 0 ? 0 : Math.max(1, Math.min(rows * 8, Math.round((total / max) * rows * 8)))
  const bounds: { color: ThemeKey; from: number; to: number }[] = []
  let sum = 0
  for (const piece of pieces) {
    const from = Math.round((sum / total) * height)
    sum += Math.max(0, piece.value)
    const to = Math.round((sum / total) * height)
    if (to > from) bounds.push({ color: piece.color, from, to })
  }
  return Array.from({ length: rows }, (_, row) => {
    const bottom = row * 8
    const filled = Math.min(8, Math.max(0, height - bottom))
    if (filled === 0) return EMPTY
    const inside = bounds.map(bound => ({
      color: bound.color,
      eighths: Math.max(0, Math.min(bound.to, bottom + filled) - Math.max(bound.from, bottom)),
    }))
    if (filled < 8) return { glyph: LOWER[filled] ?? ' ', fg: heaviest(inside), bg: null }
    const [first, ...rest] = inside.filter(piece => piece.eighths > 0)
    if (first === undefined) return EMPTY
    if (rest.length === 0) return { glyph: LOWER[8] ?? '█', fg: first.color, bg: null }
    return { glyph: LOWER[first.eighths] ?? '█', fg: first.color, bg: heaviest(rest) }
  })
}

export function bars(stacks: readonly (readonly Piece[])[], shape: Layout, rows: number, max: number): Grid {
  const grid = blank(shape.offset + shape.bars * shape.cell, rows)
  stacks.forEach((pieces, index) => {
    const cells = columnCells(pieces, max, rows)
    const left = shape.offset + shape.gap + index * shape.cell
    const width = shape.cell - shape.gap
    cells.forEach((cell, fromBottom) => {
      for (let column = 0; column < width; column += 1) put(grid, rows - 1 - fromBottom, left + column, cell)
    })
  })
  return grid
}

export function sampled(values: readonly number[], points: number): number[] {
  if (values.length === 0) return Array.from({ length: points }, () => 0)
  return Array.from({ length: points }, (_, index) => values[Math.min(values.length - 1, Math.floor((index * values.length) / points))] ?? 0)
}

export function area(values: readonly number[], columns: number, rows: number, max: number, color: ThemeKey): Grid {
  const grid = blank(columns, rows)
  const dots = sampled(values, columns * 2).map(value => (max <= 0 ? 0 : Math.min(rows * 4, Math.round((value / max) * rows * 4))))
  for (let column = 0; column < columns; column += 1) {
    const left = dots[column * 2] ?? 0
    const right = dots[column * 2 + 1] ?? 0
    for (let row = 0; row < rows; row += 1) {
      const bottom = (rows - 1 - row) * 4
      let bits = 0
      for (let dot = 0; dot < 4; dot += 1) {
        if (left > bottom + dot) bits |= LEFT_DOTS[dot] ?? 0
        if (right > bottom + dot) bits |= RIGHT_DOTS[dot] ?? 0
      }
      if (bits !== 0) put(grid, row, column, { glyph: String.fromCharCode(BRAILLE_BASE + bits), fg: color, bg: null })
    }
  }
  return grid
}

export function markers(columns: number, at: readonly number[], color: ThemeKey): Grid {
  const grid = blank(columns, 1)
  for (const column of at) put(grid, 0, column, { glyph: MARKER, fg: color, bg: null })
  return grid
}

export function stacked(top: Grid, bottom: Grid): Grid {
  return { columns: Math.max(top.columns, bottom.columns), rows: top.rows + bottom.rows, cells: [...top.cells, ...bottom.cells] }
}

export function padded(grid: Grid, columns: number): Grid {
  if (grid.columns >= columns) return grid
  return { columns, rows: grid.rows, cells: grid.cells.map(row => [...row, ...Array.from({ length: columns - grid.columns }, () => EMPTY)]) }
}

export function runs(grid: Grid): ColoredRun[][] {
  return grid.cells.map(row => {
    const merged: ColoredRun[] = []
    for (const cell of row) {
      const last = merged[merged.length - 1]
      if (last !== undefined && last.fg === cell.fg && last.bg === cell.bg) last.text += cell.glyph
      else merged.push({ text: cell.glyph, fg: cell.fg, bg: cell.bg })
    }
    return merged
  })
}

export function glyphRows(grid: Grid): string[] {
  return grid.cells.map(row => row.map(cell => cell.glyph).join(''))
}

export function encode(grid: Grid, palette: Palette): string {
  const bytes = new Uint8Array(grid.columns * grid.rows * 12)
  const view = new DataView(bytes.buffer)
  let at = 0
  for (const row of grid.cells) {
    for (const cell of row) {
      view.setUint32(at, cell.glyph.charCodeAt(0), true)
      view.setUint32(at + 4, colorOf(palette, cell.fg), true)
      view.setUint32(at + 8, colorOf(palette, cell.bg), true)
      at += 12
    }
  }
  return base64(bytes)
}

export function base64(bytes: Uint8Array): string {
  let out = ''
  for (let index = 0; index < bytes.length; index += 3) {
    const a = bytes[index] ?? 0
    const b = bytes[index + 1] ?? 0
    const c = bytes[index + 2] ?? 0
    const triple = (a << 16) | (b << 8) | c
    out += BASE64[(triple >> 18) & 63] ?? ''
    out += BASE64[(triple >> 12) & 63] ?? ''
    out += index + 1 < bytes.length ? (BASE64[(triple >> 6) & 63] ?? '') : '='
    out += index + 2 < bytes.length ? (BASE64[triple & 63] ?? '') : '='
  }
  return out
}
