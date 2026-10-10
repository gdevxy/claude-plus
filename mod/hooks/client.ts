import type { HttpInit, HttpResponse, JsonValue, SessionMeasureInput } from 'claude-code'

import type { Measure, Snapshot } from '../types'

export const TIMEOUT_MS = 500
export const OUTBOX_LIMIT = 256
export const HOST = 'http://claude-plus'
export const REBUILDING_STATUS = 503

export type Io = {
  fetch: (url: string, init: HttpInit) => Promise<HttpResponse>
  sleep: (ms: number, options: { signal: AbortSignal }) => Promise<void>
}

export type Reply = { ok: true; text: string } | { ok: false; reason: string; status: number | null }

export type SnapshotReply =
  | { kind: 'snapshot'; snapshot: Snapshot }
  | { kind: 'rebuilding' }
  | { kind: 'failed'; reason: string }

export type PlusEvent = { session: string; kind: string; ts_ms: number; data: { [key: string]: JsonValue } }

export function socketPath(home: string | undefined, plusHome: string | undefined): string | null {
  const base = plusHome !== undefined && plusHome !== '' ? plusHome : home !== undefined && home !== '' ? `${home}/.claude-plus` : null
  return base === null ? null : `${base.replace(/\/+$/, '')}/daemon.sock`
}

export async function request(io: Io, socket: string, path: string, init: HttpInit = {}): Promise<Reply> {
  const stop = new AbortController()
  const late = io.sleep(TIMEOUT_MS, { signal: stop.signal }).then(
    (): Reply => ({ ok: false, reason: `no answer within ${TIMEOUT_MS} ms`, status: null }),
    (): Reply => ({ ok: false, reason: 'cancelled', status: null }),
  )
  const answered = io.fetch(`${HOST}${path}`, { ...init, socketPath: socket }).then(
    (response): Reply =>
      response.ok
        ? { ok: true, text: response.text }
        : { ok: false, reason: `${response.status}: ${errorOf(response.text)}`, status: response.status },
    (error: unknown): Reply => ({ ok: false, reason: error instanceof Error ? error.message : String(error), status: null }),
  )
  const reply = await Promise.race([answered, late])
  stop.abort()
  return reply
}

export function snapshotPath(session: string): string {
  return `/snapshot?session=${encodeURIComponent(session)}`
}

export async function fetchSnapshot(io: Io, socket: string, session: string): Promise<SnapshotReply> {
  const reply = await request(io, socket, snapshotPath(session))
  if (!reply.ok) {
    return reply.status === REBUILDING_STATUS ? { kind: 'rebuilding' } : { kind: 'failed', reason: reply.reason }
  }
  const snapshot = parseSnapshot(reply.text)
  return snapshot === null ? { kind: 'failed', reason: 'unreadable snapshot' } : { kind: 'snapshot', snapshot }
}

export function parseSnapshot(text: string): Snapshot | null {
  const value = safeJson(text)
  if (!isObject(value) || value.v !== 1 || typeof value.session !== 'string') return null
  const live = value.live
  if (!isObject(value.totals) || !isObject(live) || !isObject(live.cache) || !Array.isArray(live.limits)) return null
  return value as unknown as Snapshot
}

export function measureOf(e: SessionMeasureInput, atMs: number): Measure {
  return {
    atMs,
    contextTokens: e.context.tokens ?? null,
    contextWindow: e.context.window,
    contextPercent: e.context.percent ?? null,
    rateLimits: e.rateLimits.map(limit => ({
      kind: limit.kind,
      percentUsed: limit.percentUsed,
      resetsAtMs: limit.resetsAt === undefined ? null : Date.parse(limit.resetsAt),
    })),
    costUsd: e.cost?.usd ?? null,
  }
}

export class Outbox {
  private queued: PlusEvent[] = []

  add(event: PlusEvent): void {
    this.queued.push(event)
    if (this.queued.length > OUTBOX_LIMIT) this.queued.splice(0, this.queued.length - OUTBOX_LIMIT)
  }

  take(): PlusEvent[] {
    const taken = this.queued
    this.queued = []
    return taken
  }

  putBack(events: PlusEvent[]): void {
    this.queued = [...events, ...this.queued].slice(-OUTBOX_LIMIT)
  }

  get size(): number {
    return this.queued.length
  }
}

export function eventsBody(events: PlusEvent[]): string {
  return JSON.stringify({ events })
}

function errorOf(text: string): string {
  const value: unknown = safeJson(text)
  return isObject(value) && typeof value.error === 'string' ? value.error : text.slice(0, 120)
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text)
  } catch {
    return undefined
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
