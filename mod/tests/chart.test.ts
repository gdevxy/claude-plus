import { describe, expect, test } from 'claude-code/testing'

import { area, bars, base64, columnCells, encode, glyphRows, grouped, layout, markers, runs, stacked } from '../hooks/chart'
import { DEFAULT_COLOR, colorOf, paletteOf } from '../hooks/palette'
import { decoded } from './paint'

describe('chart cells', () => {
  test('a layout gives every bar the same width, a gap from three columns and the spare columns on the left', () => {
    expect(layout(7, 38)).toEqual({ per: 1, bars: 7, cell: 5, gap: 1, dropped: 0, offset: 3 })
    expect(layout(24, 58)).toEqual({ per: 1, bars: 24, cell: 2, gap: 0, dropped: 0, offset: 10 })
    expect(layout(90, 38)).toEqual({ per: 3, bars: 30, cell: 1, gap: 0, dropped: 0, offset: 8 })
    expect(layout(90, 58)).toEqual({ per: 2, bars: 45, cell: 1, gap: 0, dropped: 0, offset: 13 })
    expect(layout(30, 24)).toEqual({ per: 2, bars: 15, cell: 1, gap: 0, dropped: 0, offset: 9 })
    expect(layout(7, 3)).toEqual({ per: 3, bars: 2, cell: 1, gap: 0, dropped: 1, offset: 1 })
  })

  test('merged bars keep the newest buckets and drop the oldest that do not fill a bar', () => {
    expect(grouped([1, 2, 3, 4, 5, 6, 7], layout(7, 3))).toEqual([
      [2, 3, 4],
      [5, 6, 7],
    ])
  })

  test('a stacked column draws the lower piece in the glyph and the upper one behind it', () => {
    expect(columnCells([{ value: 0.5, color: 'claude' }], 1, 2)).toEqual([
      { glyph: '█', fg: 'claude', bg: null },
      { glyph: ' ', fg: null, bg: null },
    ])
    expect(columnCells([{ value: 0.25, color: 'claude' }, { value: 0.75, color: 'permission' }], 1, 1)).toEqual([{ glyph: '▂', fg: 'claude', bg: 'permission' }])
    expect(columnCells([{ value: 0.3, color: 'claude' }], 1, 1)).toEqual([{ glyph: '▂', fg: 'claude', bg: null }])
  })

  test('any spend at all shows at least one eighth, and nothing shows nothing', () => {
    expect(columnCells([{ value: 0.001, color: 'claude' }], 1, 2)[0]).toEqual({ glyph: '▁', fg: 'claude', bg: null })
    expect(columnCells([], 1, 2)).toEqual([
      { glyph: ' ', fg: null, bg: null },
      { glyph: ' ', fg: null, bg: null },
    ])
  })

  test('bars sit at the bottom, one cell wide each, with the gap on their left', () => {
    const grid = bars([[{ value: 1, color: 'claude' }], [{ value: 0.5, color: 'claude' }], []], layout(3, 10), 2, 1)
    expect(glyphRows(grid)).toEqual(['  ██      ', '  ██ ██   '])
  })

  test('an area fills braille dots from the bottom, two values per cell', () => {
    expect(glyphRows(area([0, 4], 1, 1, 4, 'claude'))).toEqual(['⢸'])
    expect(glyphRows(area([2, 2, 4, 8], 2, 2, 8, 'claude'))).toEqual([' ⢸', '⣤⣿'])
    expect(glyphRows(stacked(markers(4, [1], 'warning'), area([0, 0, 0, 0, 0, 0, 0, 0], 4, 1, 1, 'claude')))).toEqual([' ┊  ', '    '])
  })

  test('runs merge neighbouring cells of one color into one piece of text', () => {
    const grid = bars([[{ value: 1, color: 'claude' }], [{ value: 1, color: 'claude' }], [{ value: 1, color: 'success' }]], layout(3, 3), 1, 1)
    expect(runs(grid)).toEqual([[{ text: '██', fg: 'claude', bg: null }, { text: '█', fg: 'success', bg: null }]])
  })

  test('cells encode as little-endian code point, foreground and background in padded base64', () => {
    const dark = paletteOf('dark')
    if (dark === null) throw new Error('no dark palette')
    const grid = { columns: 2, rows: 1, cells: [[{ glyph: '█', fg: 'claude' as const, bg: null }, { glyph: ' ', fg: null, bg: null }]] }
    expect(encode(grid, dark)).toBe('iCUAAFd31wAAAAABIAAAAAAAAAEAAAAB')
    expect(decoded(encode(grid, dark))).toEqual([0x2588, 0xd77757, DEFAULT_COLOR, 0x20, DEFAULT_COLOR, DEFAULT_COLOR])
    expect(base64(Uint8Array.of(1))).toBe('AQ==')
    expect(base64(Uint8Array.of(1, 2))).toBe('AQI=')
    expect(base64(Uint8Array.of(1, 2, 3))).toBe('AQID')
  })

  test('only the four built-in color themes have a palette, the rest draw text', () => {
    expect(['dark', 'light', 'dark-daltonized', 'light-daltonized'].map(name => paletteOf(name) !== null)).toEqual([true, true, true, true])
    expect(['auto', 'dark-ansi', 'light-ansi', 'custom:mine'].map(paletteOf)).toEqual([null, null, null, null])
    const light = paletteOf('light')
    if (light === null) throw new Error('no light palette')
    expect([colorOf(light, 'claude'), colorOf(light, 'text'), colorOf(light, null), colorOf(light, 'diffAdded')]).toEqual([0xd77757, 0, DEFAULT_COLOR, DEFAULT_COLOR])
  })
})
