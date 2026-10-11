import { describe, expect, test } from 'claude-code/testing'
import type { HttpInit, HttpResponse } from 'claude-code'

import type { Io, PlusEvent } from '../hooks/client'
import {
  OUTBOX_LIMIT,
  Outbox,
  breakdownOf,
  contextPath,
  eventId,
  eventsBody,
  fetchContext,
  fetchSnapshot,
  measureOf,
  parseContext,
  parseSnapshot,
  request,
  send,
  snapshotPath,
  socketPath,
  usageData,
} from '../hooks/client'
import { NOW_MS, contextReport, snapshot, usage } from './fixtures'

function answering(response: Promise<HttpResponse>, seen: [string, HttpInit][] = []): Io {
  return {
    fetch: (url, init) => {
      seen.push([url, init])
      return response
    },
    sleep: (_ms, { signal }) =>
      new Promise<void>((resolve, reject) => {
        if (signal.aborted) reject(new Error('aborted'))
        signal.addEventListener('abort', () => reject(new Error('aborted')))
      }),
  }
}

function event(n: number): PlusEvent {
  return { id: `e-${n}`, session: 's1', kind: 'turn.step', ts_ms: n, data: {} }
}

describe('daemon client', () => {
  test('the socket lives in CLAUDE_PLUS_HOME or ~/.claude-plus', () => {
    expect(socketPath('/Users/me', undefined)).toBe('/Users/me/.claude-plus/daemon.sock')
    expect(socketPath('/Users/me', '/tmp/plus/')).toBe('/tmp/plus/daemon.sock')
    expect(socketPath(undefined, undefined)).toBeNull()
  })

  test('a request goes to the Unix socket and returns the body', async () => {
    const seen: [string, HttpInit][] = []
    const io = answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: '{"v":1}' }), seen)
    expect(await request(io, '/s.sock', snapshotPath('a b'))).toEqual({ ok: true, text: '{"v":1}' })
    expect(seen).toEqual([['http://claude-plus/snapshot?session=a%20b', { socketPath: '/s.sock' }]])
  })

  test('a daemon error keeps its message and a silent daemon times out', async () => {
    const failing = answering(Promise.resolve({ status: 400, ok: false, headers: {}, text: '{"v":1,"error":"unknown kind"}' }))
    expect(await request(failing, '/s.sock', '/events')).toEqual({ ok: false, reason: '400: unknown kind', status: 400 })
    const silent: Io = { fetch: () => new Promise(() => undefined), sleep: () => Promise.resolve() }
    expect(await request(silent, '/s.sock', '/health')).toEqual({ ok: false, reason: 'no answer within 500 ms', status: null, late: true })
    const refused: Io = { fetch: () => Promise.reject(new Error('ECONNREFUSED')), sleep: () => new Promise(() => undefined) }
    expect(await request(refused, '/s.sock', '/health')).toEqual({ ok: false, reason: 'ECONNREFUSED', status: null })
  })

  test('a rebuilding daemon keeps the last good snapshot instead of failing', async () => {
    const good = JSON.stringify({ v: 1, ...snapshot() })
    const serving = answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: good }))
    const fresh = await fetchSnapshot(serving, '/s.sock', 's1')
    expect(fresh.kind).toBe('snapshot')
    const rebuilding = answering(
      Promise.resolve({ status: 503, ok: false, headers: {}, text: '{"error":"the store is being rebuilt"}' }),
    )
    expect(await fetchSnapshot(rebuilding, '/s.sock', 's1')).toEqual({ kind: 'rebuilding' })
  })

  test('other snapshot failures and unreadable bodies are reported as failures', async () => {
    const down: Io = { fetch: () => Promise.reject(new Error('ECONNREFUSED')), sleep: () => new Promise(() => undefined) }
    expect(await fetchSnapshot(down, '/s.sock', 's1')).toEqual({ kind: 'failed', reason: 'ECONNREFUSED' })
    const broken = answering(Promise.resolve({ status: 500, ok: false, headers: {}, text: '{"error":"boom"}' }))
    expect(await fetchSnapshot(broken, '/s.sock', 's1')).toEqual({ kind: 'failed', reason: '500: boom' })
    const garbled = answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: 'nope' }))
    expect(await fetchSnapshot(garbled, '/s.sock', 's1')).toEqual({ kind: 'failed', reason: 'unreadable snapshot' })
  })

  test('a snapshot from another milestone says which side is out of date', async () => {
    const { live: _live, ...older } = { v: 1, ...snapshot() }
    const serve = (body: unknown) => answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: JSON.stringify(body) }))
    expect(await fetchSnapshot(serve(older), '/s.sock', 's1')).toEqual({ kind: 'outdated', newer: 'pane' })
    expect(await fetchSnapshot(serve({ v: 2, ...snapshot() }), '/s.sock', 's1')).toEqual({ kind: 'outdated', newer: 'daemon' })
    expect(await fetchSnapshot(serve({ v: 1 }), '/s.sock', 's1')).toEqual({ kind: 'failed', reason: 'unreadable snapshot' })
  })

  test('only a version 1 snapshot with a live section is accepted', () => {
    const good = { v: 1, ...snapshot() }
    expect(parseSnapshot(JSON.stringify(good))?.live.cache.ttl).toBe('1h')
    expect(parseSnapshot(JSON.stringify({ ...good, v: 2 }))).toBeNull()
    expect(parseSnapshot(JSON.stringify({ ...good, live: undefined }))).toBeNull()
    expect(parseSnapshot('not json')).toBeNull()
  })

  test('a measure keeps context, limits and cost with resets in epoch ms', () => {
    const measure = measureOf(
      { context: { tokens: 46_000, window: 200_000, percent: 23 }, rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAt: '2026-10-11T10:00:00Z' }], cost: { usd: 4.18 }, changed: ['context'] },
      7,
    )
    expect(measure).toEqual({
      atMs: 7,
      contextTokens: 46_000,
      contextWindow: 200_000,
      contextPercent: 23,
      rateLimits: [{ kind: 'five_hour', percentUsed: 54, resetsAtMs: Date.parse('2026-10-11T10:00:00Z') }],
      costUsd: 4.18,
    })
  })

  test('a request that outlives its timeout still says how it ended', async () => {
    let answer: (response: HttpResponse) => void = () => undefined
    const slow: Io = { fetch: () => new Promise(resolve => (answer = resolve)), sleep: () => Promise.resolve() }
    const sent = send(slow, '/s.sock', '/events', { method: 'POST' })
    expect(await sent.reply).toEqual({ ok: false, reason: 'no answer within 500 ms', status: null, late: true })
    answer({ status: 200, ok: true, headers: {}, text: '{"accepted":1}' })
    expect(await sent.settled).toEqual({ ok: true, text: '{"accepted":1}' })
  })

  test('an event put back is queued once and ids count up from a prefix', () => {
    const outbox = new Outbox()
    outbox.add(event(1))
    outbox.putBack([event(1), event(2)])
    expect(outbox.take().map(queued => queued.id)).toEqual(['e-2', 'e-1'])
    expect([eventId('k3j', 1), eventId('k3j', 36)]).toEqual(['k3j-1', 'k3j-10'])
  })

  test('a connection the daemon closed is tried once more before failing', async () => {
    let calls = 0
    const resetOnce: Io = {
      fetch: () => (calls++ === 0 ? Promise.reject(new Error('ECONNRESET')) : Promise.resolve({ status: 200, ok: true, headers: {}, text: 'fine' })),
      sleep: () => new Promise(() => undefined),
    }
    expect([await request(resetOnce, '/s.sock', '/health'), calls]).toEqual([{ ok: true, text: 'fine' }, 2])
    let refused = 0
    const down: Io = {
      fetch: () => {
        refused += 1
        return Promise.reject(new Error('ECONNREFUSED'))
      },
      sleep: () => new Promise(() => undefined),
    }
    expect([await request(down, '/s.sock', '/health'), refused]).toEqual([{ ok: false, reason: 'ECONNREFUSED', status: null }, 2])
  })

  test('the outbox keeps the newest events and retries in order', () => {
    const outbox = new Outbox()
    for (let n = 0; n < OUTBOX_LIMIT + 4; n += 1) outbox.add(event(n))
    expect(outbox.size).toBe(OUTBOX_LIMIT)
    const taken = outbox.take()
    expect([taken[0]?.ts_ms, outbox.size]).toEqual([4, 0])
    outbox.add(event(999))
    outbox.putBack(taken.slice(0, 2))
    expect(outbox.take().map(queued => queued.ts_ms)).toEqual([4, 5, 999])
    expect(eventsBody([event(1)])).toBe('{"events":[{"id":"e-1","session":"s1","kind":"turn.step","ts_ms":1,"data":{}}]}')
  })

  test('a context report is fetched for the session and a rebuilding daemon is told apart', async () => {
    const seen: [string, HttpInit][] = []
    const good = JSON.stringify({ v: 1, ...contextReport() })
    const serving = answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: good }), seen)
    expect(await fetchContext(serving, '/s.sock', 'a/b')).toEqual({ kind: 'context', report: { v: 1, ...contextReport() } })
    expect(seen[0]?.[0]).toBe(`http://claude-plus${contextPath('a/b')}`)
    expect(contextPath('a/b')).toBe('/context?session=a%2Fb')
    const rebuilding = answering(Promise.resolve({ status: 503, ok: false, headers: {}, text: '{"error":"the store is being rebuilt"}' }))
    expect(await fetchContext(rebuilding, '/s.sock', 's1')).toEqual({ kind: 'rebuilding' })
    const broken = answering(Promise.resolve({ status: 500, ok: false, headers: {}, text: '{"error":"boom"}' }))
    expect(await fetchContext(broken, '/s.sock', 's1')).toEqual({ kind: 'failed', reason: '500: boom' })
    const garbled = answering(Promise.resolve({ status: 200, ok: true, headers: {}, text: '{"v":1}' }))
    expect(await fetchContext(garbled, '/s.sock', 's1')).toEqual({ kind: 'failed', reason: 'unreadable context' })
  })

  test('only a version 1 context report with its lists is accepted', () => {
    const good = { v: 1, ...contextReport() }
    expect(parseContext(JSON.stringify(good))).toEqual(good)
    expect(parseContext(JSON.stringify({ ...good, live: null }))?.live).toBeNull()
    expect(parseContext(JSON.stringify({ ...good, v: 2 }))).toBeNull()
    expect(parseContext(JSON.stringify({ ...good, items: {} }))).toBeNull()
    expect(parseContext(JSON.stringify({ ...good, config: { groups: null } }))).toBeNull()
    expect(parseContext(JSON.stringify({ ...good, live: { categories: 'x' } }))).toBeNull()
    expect(parseContext('nope')).toBeNull()
  })

  test('a full usage becomes the live breakdown the pane draws', () => {
    expect(breakdownOf(usage(), NOW_MS)).toEqual({ ...contextReport().live!, ts_ms: NOW_MS })
    const bare = usage()
    delete bare.context.breakdown
    expect(breakdownOf(bare, NOW_MS)).toBeNull()
    expect(usageData(bare)).toBeNull()
  })

  test('the forwarded usage keeps what the daemon reads and drops the grid', () => {
    const data = usageData(usage())
    expect(data).not.toBeNull()
    expect(Object.keys(data!)).toEqual([
      'model',
      'totalTokens',
      'maxTokens',
      'rawMaxTokens',
      'percentage',
      'autocompactSource',
      'autoCompactThreshold',
      'isAutoCompactEnabled',
      'categories',
      'memoryFiles',
      'mcpTools',
      'agents',
      'skills',
      'slashCommands',
      'apiUsage',
      'contextTokens',
      'contextWindow',
    ])
    expect(data!.skills).toEqual({
      totalSkills: 2,
      includedSkills: 2,
      tokens: 2_600,
      skillFrontmatter: [{ name: 'superpowers:brainstorming', source: 'plugin', pluginName: 'superpowers', tokens: 2_300 }],
    })
    expect(data!.slashCommands).toBeNull()
    expect(JSON.parse(JSON.stringify(data))).toEqual(data)
  })
})
