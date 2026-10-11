import { describe, expect, test } from 'claude-code/testing'

import type { ContextInput } from '../hooks/context'
import { compositionOf, contextScreen, themeColor } from '../hooks/context'
import type { Line } from '../hooks/state'
import { WHY_COLUMNS } from '../hooks/state'
import { MINUTE_MS, NOW_MS, contextReport, local } from './fixtures'

const OK = { status: 'ok', detail: null } as const

function input(overrides: Partial<ContextInput> = {}): ContextInput {
  return { width: 40, nowMs: NOW_MS, report: contextReport(), local: local(), daemon: OK, expanded: [], ...overrides }
}

function text(line: Line): string {
  return line.segments.map(segment => segment.text).join('')
}

function frame(lines: Line[]): string[] {
  return lines.map(line => `${text(line)}${line.why === undefined ? '' : ' ?'}`)
}

function line(lines: Line[], key: string): Line {
  const found = lines.find(candidate => candidate.key === key)
  if (found === undefined) throw new Error(`no line ${key}`)
  return found
}

describe('context screen', () => {
  test('draws the live breakdown, the largest items and the setup at 40 columns', () => {
    expect(frame(contextScreen(input()))).toEqual([
      'Context · Opus 5.5          46k / 200k',
      '███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
      'Auto-compact at 167k · 121k to go',
      ' ',
      'By category                          ?',
      '  System prompt █░░░░░░░░░░░░░    3.1k',
      '  System tools  ████▍░░░░░░░░░   14.2k',
      '▸ MCP tools     █▎░░░░░░░░░░░░    4.1k',
      '▸ Memory files  ▋░░░░░░░░░░░░░      2k',
      '▸ Skills        ▊░░░░░░░░░░░░░    2.6k',
      '  Messages      ██████▏░░░░░░░     20k',
      '  Free space                      121k',
      '  Autocompact buffer               33k',
      '+ 8.4k of tool schemas load on demand',
      '~12.4k injected since your first request · 1 compaction',
      ' ',
      'Largest items                        ?',
      '  MCP linear · 2 tools            4.1k',
      '  Skill superpowers:brainstorming 2.3k',
      '  CLAUDE.md · Project             1.6k',
      '  Agent reviewer                   120',
      ' ',
      'Not used in 14 days        2.3k a turn',
      '  Skill superpowers:brainstorming 2.3k',
      ' ',
      'Largest tool results                  ',
      '  Read                            ~12k',
      '  linear search                  ~4.2k',
      ' ',
      'Configured                           ?',
      '  CLAUDE.md files      3 · 1 on demand',
      '  Skills              66 · 2 on demand',
      '  MCP servers                        3',
      '  Plugins                    5 · 1 off',
    ])
  })

  for (const width of [40, 60, 100]) {
    test(`every unwrapped line fills exactly ${width - 2} columns`, () => {
      const expanded = ['categories', 'estimate', 'items', 'config', 'open-memory_file', 'open-skill', 'open-mcp_server']
      for (const report of [contextReport(), contextReport({ live: null })]) {
        const lines = contextScreen(input({ width, report, expanded }))
        for (const drawn of lines.filter(candidate => candidate.wrap !== true && !candidate.key.startsWith('gap-'))) {
          expect([drawn.key, text(drawn).length + (drawn.why === undefined ? 0 : WHY_COLUMNS)]).toEqual([drawn.key, width - 2])
        }
      }
    })
  }

  test('estimates the composition from the transcript until a breakdown arrives', () => {
    const lines = contextScreen(input({ report: contextReport({ live: null }), expanded: ['estimate'] }))
    expect(frame(lines).slice(0, 11)).toEqual([
      'Context                     46k / 200k',
      '███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
      ' ',
      'Estimated from the transcript        ?',
      '  System and tools ████░░░░░░░  ~16.9k',
      '▸ Memory files     ▍░░░░░░░░░░   ~1.5k',
      '▸ Skills           ▌░░░░░░░░░░     ~2k',
      '  Messages         ██████▏░░░░  ~25.6k',
      '~12.4k injected since your first request · 1 compaction',
      'Text Claude Code injected, at 3.5 characters per token (estimated until Claude Code counts your memory files), fitted to the 20k tokens of your first request. The rest of each request is messages.',
      ' ',
    ])
    expect(line(lines, 'ctx-part-memory').segments[1]?.color).toBe('autoAccept')
    expect(text(line(lines, 'ctx-top-0'))).toBe('  MCP linear · 2 tools            4.1k')
  })

  test('names the memory files Claude Code counted once they calibrate the estimate', () => {
    const calibrated = contextReport({ live: null, ratios: { prose: 4, code: 3.4, schema: 2.9, calibrated: true, pairs: 3 } })
    const lines = contextScreen(input({ report: calibrated, expanded: ['estimate'] }))
    expect(text(line(lines, 'explain-estimate'))).toBe(
      'Text Claude Code injected, at 4.0 characters per token (calibrated on 3 memory files Claude Code counted), fitted to the 20k tokens of your first request. The rest of each request is messages.',
    )
  })

  test('the newest of the local and the stored breakdown wins, and another session is ignored', () => {
    const report = contextReport()
    const stored = report.live!
    const newer = { ...stored, ts_ms: stored.ts_ms + MINUTE_MS, total_tokens: 50_000 }
    expect(compositionOf({ report, local: local({ breakdown: newer }) })?.total_tokens).toBe(50_000)
    const older = { ...stored, ts_ms: stored.ts_ms - MINUTE_MS, total_tokens: 40_000 }
    expect(compositionOf({ report, local: local({ breakdown: older }) })?.total_tokens).toBe(46_000)
    const elsewhere = contextReport({ session: 's2' })
    expect(compositionOf({ report: elsewhere, local: local() })).toBeNull()
    expect(compositionOf({ report: elsewhere, local: local({ breakdown: older }) })?.total_tokens).toBe(40_000)
  })

  test('keeps the live breakdown from Claude Code alone while the daemon is missing', () => {
    const breakdown = contextReport().live
    const lines = contextScreen(input({ report: null, local: local({ breakdown }), daemon: { status: 'missing', detail: null } }))
    expect(frame(lines).slice(0, 7)).toEqual([
      'claude-plus is not installed yet. Install it, or set its path in the plugin options',
      ' ',
      'Context · Opus 5.5          46k / 200k',
      '███████▋░░░░░░░░░░░░░░░░░░░░░░░░░  23%',
      'Auto-compact at 167k · 121k to go',
      ' ',
      'By category                          ?',
    ])
    expect(lines.some(candidate => candidate.key.startsWith('ctx-top'))).toBe(false)
    expect(lines.some(candidate => candidate.key === 'ctx-config')).toBe(false)
  })

  test('greets a fresh session before the first prompt', () => {
    const fresh = local({ measure: null })
    expect(frame(contextScreen(input({ report: null, local: fresh })))).toEqual(['Context', 'The breakdown appears after your first prompt ✻'])
  })

  test('says when auto-compaction is due and explains each section once asked', () => {
    const report = contextReport()
    const full = contextReport({ live: { ...report.live!, total_tokens: 170_000, percentage: 85 } })
    const lines = contextScreen(input({ report: full, expanded: ['categories', 'items', 'config'] }))
    expect(line(lines, 'ctx-compact')).toEqual({
      key: 'ctx-compact',
      segments: [{ text: 'Auto-compact at 167k · due now', color: 'warning' }],
      wrap: true,
    })
    expect(line(lines, 'ctx-bar').segments[1]?.color).toBe('warning')
    expect(text(line(lines, 'explain-categories'))).toBe(
      "Claude Code's own /context breakdown, counted 2m ago after a turn. Bars share what is in use; free space and the compaction buffer are the window left. A row with ▸ opens its items.",
    )
    expect(text(line(lines, 'explain-items'))).toBe(
      'What your setup loads into every request, counted by Claude Code. Skills and agents cost their listing line until used.',
    )
    expect(text(line(lines, 'explain-config'))).toBe(
      "Read from disk <1m ago, never changed. On-demand files load when Claude works in their folder; an MCP server's size is known once a session has loaded it.",
    )
  })

  test('a category row opens the items Claude Code counted in it', () => {
    const closed = contextScreen(input())
    expect(line(closed, 'ctx-cat-4').toggle).toBe('open-memory_file')
    expect(line(closed, 'ctx-cat-0').toggle).toBeUndefined()
    const report = contextReport()
    const skills = Array.from({ length: 10 }, (_, index) => ({
      kind: 'skill' as const,
      name: `tidy-${index}`,
      detail: index === 0 ? 'superpowers' : null,
      tokens: 300 - index * 10,
      estimated: false,
      uses_session: 0,
      uses_14d: 1,
    }))
    const open = contextScreen(input({ report: { ...report, items: [...report.items, ...skills] }, expanded: ['open-memory_file', 'open-skill'] }))
    expect(frame(open).slice(8, 21)).toEqual([
      '▾ Memory files  ▋░░░░░░░░░░░░░      2k',
      '    CLAUDE.md · Project           1.6k',
      '▾ Skills        ▊░░░░░░░░░░░░░    2.6k',
      '    superpowers:brainstorming     2.3k',
      '    tidy-0                         300',
      '    tidy-1                         290',
      '    tidy-2                         280',
      '    tidy-3                         270',
      '    tidy-4                         260',
      '    tidy-5                         250',
      '    tidy-6                         240',
      '    + 3 more                       660',
      '  Messages      ██████▏░░░░░░░     20k',
    ])
    const estimated = contextScreen(input({ report: contextReport({ live: null }), expanded: ['open-memory_file'] }))
    expect(text(line(estimated, 'ctx-part-memory-item-0'))).toBe('    CLAUDE.md · Project           1.6k')
  })

  test('says how many unused items are left out and what they cost', () => {
    const report = contextReport()
    const unused = Array.from({ length: 6 }, (_, index) => ({
      kind: 'skill' as const,
      name: `idle-${index}`,
      detail: null,
      tokens: 600 - index * 100,
      estimated: false,
      uses_session: 0,
      uses_14d: 0,
    }))
    const lines = contextScreen(input({ report: { ...report, dead_weight: unused } }))
    const at = frame(lines).indexOf('Not used in 14 days        2.1k a turn')
    expect(frame(lines).slice(at, at + 6)).toEqual([
      'Not used in 14 days        2.1k a turn',
      '  Skill idle-0                     600',
      '  Skill idle-1                     500',
      '  Skill idle-2                     400',
      '  Skill idle-3                     300',
      '  + 2 more                         300',
    ])
  })

  test('colors categories with Claude Code theme keys and falls back to text', () => {
    expect(themeColor('permission')).toBe('permission')
    expect(themeColor('purple_FOR_SUBAGENTS_ONLY')).toBe('text')
    const lines = contextScreen(input())
    expect(line(lines, 'ctx-cat-2').segments[1]).toEqual({ text: '█▎', color: 'permission' })
    expect(line(lines, 'ctx-cat-6').segments[1]?.color).toBe('text')
    expect(line(lines, 'ctx-cat-7').segments.every(segment => segment.dim === true)).toBe(true)
  })
})
