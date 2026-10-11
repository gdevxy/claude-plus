export type PlusView = 'live' | 'context' | 'history' | 'insights'

export type Tokens = {
  input: number
  output: number
  thinking: number
  cache_read: number
  cache_write_5m: number
  cache_write_1h: number
}

export type Totals = {
  cost_usd: number
  reconciled_usd: number
  unpriced_requests: number
  requests: number
  subagents: number
  last_request_ms: number | null
  tokens: Tokens
}

export type CacheName = 'warm' | 'cold' | 'none'

export type Rebuild = {
  ts_ms: number
  cause: 'idle' | 'model_switch' | 'prefix_changed' | 'unknown'
  tokens: number
  extra_usd: number | null
}

export type Cache = {
  state: CacheName
  ttl: '5m' | '1h'
  ttl_source: 'observed' | 'assumed'
  last_touch_ms: number | null
  expires_at_ms: number | null
  hit_ratio: number | null
  saved_usd: number
  rebuilds: number
  rebuild_usd: number
  last_rebuild: Rebuild | null
}

export type Forecast = {
  model: string
  samples: number
  prefix_tokens: number
  new_tokens: number
  output_tokens: number
  warm_usd: number | null
  cold_usd: number | null
  warm_pct_5h: number | null
  cold_pct_5h: number | null
  lineup: ModelCost[]
}

export type ModelCost = { model: string; warm_usd: number; cold_usd: number }

export type LimitWindow = {
  window: string
  used_pct: number
  resets_at_ms: number | null
  sampled_at_ms: number
  reset_since_sample: boolean
  pace: { pct_per_hour: number; projected_pct: number | null; full_at_ms: number | null } | null
  calibration: { family: string; pairs: number; needed: number; pct_per_usd: number | null }
}

export type AccountRef = { id: string; label: string; kind: 'oauth' | 'api' | 'cloud' | 'gateway' }

export type BudgetView = { amount_usd: number; local_usd: number; period_start_ms: number; resets_at_ms: number }

export type Live = {
  model: string | null
  effort: string | null
  cache: Cache
  forecast: Forecast | null
  limits: LimitWindow[]
  account: AccountRef | null
  budget: BudgetView | null
}

export type Snapshot = {
  session: string
  known: boolean
  generated_at_ms: number
  totals: Totals
  live: Live
}

export type RateLimit = { kind: string; percentUsed: number; resetsAtMs: number | null }

export type Measure = {
  atMs: number
  contextTokens: number | null
  contextWindow: number
  contextPercent: number | null
  rateLimits: RateLimit[]
  costUsd: number | null
}

export type Step = {
  atMs: number
  model: string
  input: number
  output: number
  cacheRead: number
  cacheWrite: number
}

export type Local = {
  session: string | null
  model: string | null
  effort: string | null
  turns: number
  working: boolean
  measure: Measure | null
  step: Step | null
  breakdown: LiveContext | null
}

export type CategoryKind = 'used' | 'free' | 'buffer' | 'deferred'

export type BreakdownCategory = { name: string; tokens: number; color: string; kind: CategoryKind; item_kind?: ItemKind }

export type LiveContext = {
  ts_ms: number
  model: string
  total_tokens: number
  max_tokens: number
  percentage: number
  auto_compact_threshold: number | null
  categories: BreakdownCategory[]
}

export type Part = 'system' | 'memory' | 'skills' | 'mcp' | 'agents' | 'files' | 'hooks' | 'reminders' | 'messages'

export type Share = { part: Part; tokens: number }

export type HistoryPoint = { ts_ms: number; prompt_tokens: number; injected_tokens: number; compacted: boolean }

export type History = {
  first_request_ms: number
  measured_tokens: number
  first_message_tokens: number
  scale: number
  initial: Share[]
  now: Share[]
  injected_later_tokens: number
  compactions: number
  points: HistoryPoint[]
}

export type Ratios = { prose: number; code: number; schema: number; calibrated: boolean; pairs: number }

export type ItemKind = 'memory_file' | 'skill' | 'mcp_server' | 'agent' | 'output_style'

export type ContextItem = {
  kind: ItemKind
  name: string
  detail: string | null
  tokens: number
  estimated: boolean
  uses_session: number | null
  uses_14d: number | null
}

export type ToolResult = { tool: string | null; ts_ms: number; chars: number; images: number; tokens: number }

export type SourceKind = 'claude_md' | 'memory' | 'skill' | 'agent' | 'output_style' | 'mcp_server' | 'plugin'

export type Loading = 'always' | 'listing' | 'on_demand' | 'off' | 'unknown'

export type ConfigGroup = { kind: SourceKind; loading: Loading; count: number; tokens: number }

export type ContextReport = {
  session: string
  generated_at_ms: number
  project_dir: string | null
  live: LiveContext | null
  history: History | null
  ratios: Ratios
  items: ContextItem[]
  dead_weight: ContextItem[]
  tool_results: ToolResult[]
  config: { scanned_at_ms: number; groups: ConfigGroup[] }
}

export type HistoryRange = '24h' | '7d' | '30d' | '90d'

export type Metric = 'cost' | 'tokens' | 'pct'

export type Spend = {
  requests: number
  unpriced: number
  cost_usd: number
  reconciled_usd: number
  tokens: number
  pct_5h: number | null
}

export type ModelSpend = Spend & { model: string }

export type Bucket = Spend & {
  start_ms: number
  label: string
  models: ModelSpend[]
  prompt_tokens: number
  cache_read_tokens: number
  rebuilds: number
  rebuild_usd: number
  compactions: number
  peak_5h_pct: number | null
}

export type HistoryTotals = Spend & {
  sessions: number
  hit_ratio: number | null
  saved_usd: number
  rebuilds: number
  rebuild_usd: number
  idle_rebuilds: number
  compactions: number
  peak_5h_pct: number | null
}

export type ProjectSpend = Spend & { name: string; path: string | null; sessions: number }

export type SessionSpend = Spend & { id: string; title: string | null; project: string; last_ms: number }

export type HistoryReport = {
  range: HistoryRange
  bucket: 'hour' | 'day'
  generated_at_ms: number
  since_ms: number
  until_ms: number
  calibration: { calibrated: boolean; pairs: number; needed: number }
  totals: HistoryTotals
  buckets: Bucket[]
  models: ModelSpend[]
  projects: ProjectSpend[]
  sessions: SessionSpend[]
}

export type Reports = Partial<Record<HistoryRange, HistoryReport>>

export type HistoryFailure = { kind: 'missing' | 'failed'; reason: string }

export type Failures = Partial<Record<HistoryRange, HistoryFailure>>

export type Chart = { range: HistoryRange; metric: Metric }

export type RuleId =
  | 'idle_rebuilds'
  | 'unused_mcp_servers'
  | 'unused_skills'
  | 'oversized_memory'
  | 'large_tool_results'
  | 'late_compaction'
  | 'model_fit'
  | 'cache_busts'
  | 'costly_tasks'
  | 'retention'

export type Confidence = 'high' | 'medium' | 'low'

export type Finding = {
  rule: RuleId
  tip: string
  evidence: string[]
  saving_usd: number | null
  saving_pct_5h: number | null
  confidence: Confidence
  how: string
}

export type Wins = { saved_usd_7d: number; hit_streak_days: number }

export type Recap = { day: string; start_ms: number; cost_usd: number; hit_ratio: number | null; saved_usd: number; win: string | null }

export type InsightsReport = {
  session: string
  generated_at_ms: number
  since_ms: number
  requests: number
  sessions: number
  findings: Finding[]
  wins: Wins
  recap: Recap | null
}

export type InsightsFailure = HistoryFailure

export type Alerted = {
  session: string | null
  model: string | null
  cacheFor: number | null
  paceFor: number | null
  contextFired: boolean
  threshold: number | null
  thresholdAsked: boolean
}

export type DaemonStatus = 'starting' | 'ok' | 'rebuilding' | 'unreachable' | 'stopped' | 'outdated' | 'behind' | 'newer' | 'missing' | 'failed'

export type Daemon = { status: DaemonStatus; detail: string | null }

export type Motion = { from: number; to: number; startMs: number }

export type PaneState = { session: string | null; asked: boolean; unasked: 'waiting' | 'settled' }

declare module 'claude-code' {
  interface PluginState {
    plus: {
      view: PlusView
      snapshot: Shaped<Snapshot | null>
      context: Shaped<ContextReport | null>
      history: Shaped<Reports>
      failures: Failures
      chart: Chart
      insights: Shaped<InsightsReport | null>
      insightsFailure: InsightsFailure | null
      alerted: Shaped<Alerted>
      theme: string
      local: Shaped<Local>
      pane: PaneState
      daemon: Daemon
      now: number
      expanded: string[]
      motions: Record<string, Motion>
    }
  }
}
