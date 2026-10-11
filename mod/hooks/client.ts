import type { HttpInit, HttpResponse, JsonValue, SessionMeasureInput, SessionUsage } from 'claude-code'

import type { BreakdownCategory, ContextReport, HistoryRange, HistoryReport, InsightsReport, LiveContext, Measure, Snapshot } from '../types'
import { itemKinds } from './context'

export const TIMEOUT_MS = 500
export const HISTORY_TIMEOUT_MS = 3000
export const INSIGHTS_TIMEOUT_MS = 3000
export const OUTBOX_LIMIT = 256
export const HOST = 'http://claude-plus'
export const REBUILDING_STATUS = 503
export const NOT_FOUND_STATUS = 404

export type Io = {
  fetch: (url: string, init: HttpInit) => Promise<HttpResponse>
  sleep: (ms: number, options: { signal: AbortSignal }) => Promise<void>
}

export type Reply = { ok: true; text: string } | { ok: false; reason: string; status: number | null; late?: true }

export type Sent = { reply: Promise<Reply>; settled: Promise<Reply> }

export type SnapshotReply =
  | { kind: 'snapshot'; snapshot: Snapshot }
  | { kind: 'rebuilding' }
  | { kind: 'outdated'; newer: 'daemon' | 'pane' }
  | { kind: 'failed'; reason: string }

export type ContextReply =
  | { kind: 'context'; report: ContextReport }
  | { kind: 'rebuilding' }
  | { kind: 'failed'; reason: string }

export type HistoryReply =
  | { kind: 'history'; report: HistoryReport }
  | { kind: 'rebuilding' }
  | { kind: 'missing' }
  | { kind: 'failed'; reason: string }

export type InsightsReply =
  | { kind: 'insights'; report: InsightsReport }
  | { kind: 'rebuilding' }
  | { kind: 'missing' }
  | { kind: 'failed'; reason: string }

export type PlusEvent = { id: string; session: string; kind: string; ts_ms: number; data: { [key: string]: JsonValue } }

export function socketPath(home: string | undefined, plusHome: string | undefined): string | null {
  const base = plusHome !== undefined && plusHome !== '' ? plusHome : home !== undefined && home !== '' ? `${home}/.claude-plus` : null
  return base === null ? null : `${base.replace(/\/+$/, '')}/daemon.sock`
}

export function request(io: Io, socket: string, path: string, init: HttpInit = {}, timeoutMs = TIMEOUT_MS): Promise<Reply> {
  return send(io, socket, path, init, timeoutMs).reply
}

export function send(io: Io, socket: string, path: string, init: HttpInit = {}, timeoutMs = TIMEOUT_MS): Sent {
  const stop = new AbortController()
  const late = io.sleep(timeoutMs, { signal: stop.signal }).then(
    (): Reply => ({ ok: false, reason: `no answer within ${timeoutMs} ms`, status: null, late: true }),
    (): Reply => ({ ok: false, reason: 'cancelled', status: null }),
  )
  const attempt = () => io.fetch(`${HOST}${path}`, { ...init, socketPath: socket })
  const answered = attempt()
    .catch((error: unknown) => (stop.signal.aborted ? Promise.reject(error) : attempt()))
    .then(
      (response): Reply =>
        response.ok
          ? { ok: true, text: response.text }
          : { ok: false, reason: `${response.status}: ${errorOf(response.text)}`, status: response.status },
      (error: unknown): Reply => ({ ok: false, reason: error instanceof Error ? error.message : String(error), status: null }),
    )
  const reply = Promise.race([answered, late]).then(first => {
    stop.abort()
    return first
  })
  return { reply, settled: answered }
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
  if (snapshot !== null) return { kind: 'snapshot', snapshot }
  const newer = outdatedSide(reply.text)
  return newer === null ? { kind: 'failed', reason: 'unreadable snapshot' } : { kind: 'outdated', newer }
}

function outdatedSide(text: string): 'daemon' | 'pane' | null {
  const value = safeJson(text)
  if (!isObject(value) || typeof value.v !== 'number') return null
  if (value.v > 1) return 'daemon'
  return value.v === 1 && typeof value.session === 'string' && isObject(value.totals) && !isObject(value.live) ? 'pane' : null
}

export function parseSnapshot(text: string): Snapshot | null {
  const value = safeJson(text)
  if (!isObject(value) || value.v !== 1 || typeof value.session !== 'string') return null
  const live = value.live
  if (!isObject(value.totals) || !isObject(live) || !isObject(live.cache) || !Array.isArray(live.limits)) return null
  if (isObject(live.forecast) && !Array.isArray(live.forecast.lineup)) live.forecast.lineup = []
  return value as unknown as Snapshot
}

export function contextPath(session: string): string {
  return `/context?session=${encodeURIComponent(session)}`
}

export async function fetchContext(io: Io, socket: string, session: string): Promise<ContextReply> {
  const reply = await request(io, socket, contextPath(session))
  if (!reply.ok) {
    return reply.status === REBUILDING_STATUS ? { kind: 'rebuilding' } : { kind: 'failed', reason: reply.reason }
  }
  const report = parseContext(reply.text)
  return report === null ? { kind: 'failed', reason: 'unreadable context' } : { kind: 'context', report }
}

export function parseContext(text: string): ContextReport | null {
  const value = safeJson(text)
  if (!isObject(value) || value.v !== 1 || typeof value.session !== 'string') return null
  const lists = [value.items, value.dead_weight, value.tool_results]
  if (!lists.every(Array.isArray) || !isObject(value.config) || !Array.isArray(value.config.groups)) return null
  if (value.live !== null && !(isObject(value.live) && Array.isArray(value.live.categories))) return null
  return value as unknown as ContextReport
}

export function historyPath(range: HistoryRange): string {
  return `/history?range=${range}`
}

export async function fetchHistory(io: Io, socket: string, range: HistoryRange): Promise<HistoryReply> {
  const reply = await request(io, socket, historyPath(range), {}, HISTORY_TIMEOUT_MS)
  if (!reply.ok) {
    if (reply.status === NOT_FOUND_STATUS) return { kind: 'missing' }
    return reply.status === REBUILDING_STATUS ? { kind: 'rebuilding' } : { kind: 'failed', reason: reply.reason }
  }
  const report = parseHistory(reply.text)
  return report === null ? { kind: 'failed', reason: 'unreadable history' } : { kind: 'history', report }
}

export function parseHistory(text: string): HistoryReport | null {
  const value = safeJson(text)
  if (!isObject(value) || value.v !== 1 || typeof value.range !== 'string' || !isObject(value.totals)) return null
  const lists = [value.buckets, value.models, value.projects, value.sessions]
  if (!lists.every(Array.isArray) || !isObject(value.calibration)) return null
  return value as unknown as HistoryReport
}

export function insightsPath(session: string): string {
  return `/insights?session=${encodeURIComponent(session)}`
}

export async function fetchInsights(io: Io, socket: string, session: string): Promise<InsightsReply> {
  const reply = await request(io, socket, insightsPath(session), {}, INSIGHTS_TIMEOUT_MS)
  if (!reply.ok) {
    if (reply.status === NOT_FOUND_STATUS) return { kind: 'missing' }
    return reply.status === REBUILDING_STATUS ? { kind: 'rebuilding' } : { kind: 'failed', reason: reply.reason }
  }
  const report = parseInsights(reply.text)
  return report === null ? { kind: 'failed', reason: 'unreadable insights' } : { kind: 'insights', report }
}

export function parseInsights(text: string): InsightsReport | null {
  const value = safeJson(text)
  if (!isObject(value) || value.v !== 1 || typeof value.session !== 'string' || typeof value.requests !== 'number') return null
  if (!Array.isArray(value.findings) || !value.findings.every(isObject) || !isObject(value.wins)) return null
  if (value.recap !== null && !isObject(value.recap)) return null
  return value as unknown as InsightsReport
}

export function breakdownOf(usage: SessionUsage, atMs: number): LiveContext | null {
  const breakdown = usage.context.breakdown
  if (breakdown === undefined) return null
  const categories: BreakdownCategory[] = breakdown.categories.map(category => ({
    name: category.name,
    tokens: category.tokens,
    color: category.color,
    kind: category.isDeferred ? 'deferred' : category.kind,
  }))
  const sum = (values: readonly number[]) => values.reduce((total, value) => total + value, 0)
  const kinds = itemKinds(categories, {
    memory_file: sum(breakdown.memoryFiles.map(file => file.tokens)),
    mcp_server: sum(breakdown.mcpTools.filter(tool => tool.isLoaded).map(tool => tool.tokens)),
    skill: breakdown.skills?.tokens ?? 0,
    agent: sum(breakdown.agents.map(agent => agent.tokens)),
  })
  return {
    ts_ms: atMs,
    model: breakdown.model,
    total_tokens: breakdown.totalTokens,
    max_tokens: Math.max(breakdown.rawMaxTokens, breakdown.maxTokens),
    percentage: breakdown.percentage,
    auto_compact_threshold: breakdown.autoCompactThreshold ?? null,
    categories: categories.map((category, index) => {
      const kind = kinds[index]
      return kind === undefined ? category : { ...category, item_kind: kind }
    }),
  }
}

export function usageData(usage: SessionUsage): { [key: string]: JsonValue } | null {
  const b = usage.context.breakdown
  if (b === undefined) return null
  return {
    model: b.model,
    totalTokens: b.totalTokens,
    maxTokens: b.maxTokens,
    rawMaxTokens: b.rawMaxTokens,
    percentage: b.percentage,
    autocompactSource: b.autocompactSource,
    autoCompactThreshold: b.autoCompactThreshold ?? null,
    isAutoCompactEnabled: b.isAutoCompactEnabled,
    categories: b.categories.map(c => ({ name: c.name, tokens: c.tokens, color: c.color, isDeferred: c.isDeferred, kind: c.kind })),
    memoryFiles: b.memoryFiles.map(f => ({ path: f.path, type: f.type, tokens: f.tokens })),
    mcpTools: b.mcpTools.map(t => ({ name: t.name, serverName: t.serverName, tokens: t.tokens, isLoaded: t.isLoaded })),
    agents: b.agents.map(a => ({ agentType: a.agentType, source: a.source, tokens: a.tokens })),
    skills:
      b.skills === undefined
        ? null
        : {
            totalSkills: b.skills.totalSkills,
            includedSkills: b.skills.includedSkills,
            tokens: b.skills.tokens,
            skillFrontmatter: b.skills.skillFrontmatter.map(skill => ({
              name: skill.name,
              source: skill.source,
              pluginName: skill.pluginName ?? null,
              tokens: skill.tokens,
            })),
          },
    slashCommands:
      b.slashCommands === undefined
        ? null
        : { totalCommands: b.slashCommands.totalCommands, includedCommands: b.slashCommands.includedCommands, tokens: b.slashCommands.tokens },
    apiUsage: b.apiUsage === null ? null : { ...b.apiUsage },
    contextTokens: usage.context.tokens ?? null,
    contextWindow: usage.context.window,
  }
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
    const queued = new Set(this.queued.map(event => event.id))
    this.queued = [...events.filter(event => !queued.has(event.id)), ...this.queued].slice(-OUTBOX_LIMIT)
  }

  get size(): number {
    return this.queued.length
  }
}

export function eventId(prefix: string, sequence: number): string {
  return `${prefix}-${sequence.toString(36)}`
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
