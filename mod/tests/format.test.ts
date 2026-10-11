import { describe, expect, test } from 'claude-code/testing'

import {
  approx,
  bar,
  countdown,
  duration,
  finePercent,
  fit,
  modelName,
  money,
  percent,
  spread,
  tokens,
  whole,
  windowName,
} from '../hooks/format'

describe('format', () => {
  test('a budget amount drops its cents only when it has none', () => {
    expect([700, 23.1, 12_500, 1234.5, 999.999, 0.5].map(whole)).toEqual(['$700', '$23.10', '$12,500', '$1,234.50', '$1,000', '$0.50'])
  })

  test('money keeps cents, flags tiny amounts and groups thousands', () => {
    expect([0, 0.004, 0.01, 4.181, 999.994, 1234.5].map(money)).toEqual([
      '$0.00',
      '<$0.01',
      '$0.01',
      '$4.18',
      '$999.99',
      '$1,235',
    ])
  })

  test('approx marks estimates without doubling up on a tiny amount', () => {
    expect([0, 0.004, 0.34].map(approx)).toEqual(['~$0.00', '<$0.01', '~$0.34'])
  })

  test('tokens scale to k and M with one decimal below a hundred and no trailing zero', () => {
    expect([842, 1000, 82_149, 182_400, 999_960, 2_400_000, 123_456_789].map(tokens)).toEqual([
      '842',
      '1k',
      '82.1k',
      '182k',
      '1M',
      '2.4M',
      '123M',
    ])
  })

  test('tokens past a million scale on to B and T', () => {
    expect([999_960_000, 1_178_000_000, 3_379_000_000, 18_408_000_000, 124_500_000_000, 999_960_000_000, 2_400_000_000_000, 1_234_000_000_000_000].map(tokens)).toEqual([
      '1B',
      '1.2B',
      '3.4B',
      '18.4B',
      '125B',
      '1T',
      '2.4T',
      '1234T',
    ])
  })

  test('percent rounds and never shows a used window as zero', () => {
    expect([0, 0.4, 54.4, 54.5, 100].map(percent)).toEqual(['0%', '<1%', '54%', '55%', '100%'])
    expect([0, 0.04, 0.42, 9.94, 12.4].map(finePercent)).toEqual(['0.0%', '<0.1%', '0.4%', '9.9%', '12%'])
  })

  test('countdown and duration read like a clock and a sentence', () => {
    expect([-5, 0, 999, 61_000, 3_134_000, 3_600_000].map(countdown)).toEqual([
      '0:00',
      '0:00',
      '0:01',
      '1:01',
      '52:14',
      '1:00:00',
    ])
    expect([30_000, 45 * 60_000, 2 * 3_600_000, 8_040_000, 3 * 86_400_000 + 4 * 3_600_000].map(duration)).toEqual([
      '<1m',
      '45m',
      '2h',
      '2h 14m',
      '3d 4h',
    ])
  })

  test('model ids become short names', () => {
    expect(
      ['claude-opus-5-5', 'claude-haiku-4-5-20251001', 'claude-sonnet-5-5[1m]', 'claude-fable-5-1', 'gpt-x'].map(modelName),
    ).toEqual(['Opus 5.5', 'Haiku 4.5', 'Sonnet 5.5', 'Fable 5.1', 'gpt-x'])
    expect(['five_hour', 'seven_day', 'spend_limit', 'other'].map(windowName)).toEqual(['5h', '7d', 'Spend', 'other'])
  })

  test('spread right-aligns the value and cuts the label, never the value', () => {
    expect(spread('Cost', '$4.18', 12)).toBe('Cost   $4.18')
    expect(spread('A long label here', '$4.18', 12)).toBe('A lon… $4.18')
    expect(spread('Exactly', '$4.18', 13)).toBe('Exactly $4.18')
    expect(spread('Exactly!', '$4.18', 13)).toBe('Exactl… $4.18')
    expect(fit('abc', 2)).toBe('a…')
  })

  test('bars fill in eighths and always span their width', () => {
    expect(bar(0, 4)).toEqual({ filled: '', track: '░░░░' })
    expect(bar(0.5, 4)).toEqual({ filled: '██', track: '░░' })
    expect(bar(0.53, 4)).toEqual({ filled: '██▏', track: '░' })
    expect(bar(1, 4)).toEqual({ filled: '████', track: '' })
    expect(bar(7, 4)).toEqual({ filled: '████', track: '' })
    expect(bar(Number.NaN, 2)).toEqual({ filled: '', track: '░░' })
  })
})
