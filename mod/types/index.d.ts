export type PlusView = 'live' | 'context'

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
  alternative: { model: string; first_usd: number; next_usd: number } | null
}

export type LimitWindow = {
  window: string
  used_pct: number
  resets_at_ms: number | null
  sampled_at_ms: number
  reset_since_sample: boolean
  pace: { pct_per_hour: number; projected_pct: number | null; full_at_ms: number | null } | null
  calibration: { family: string; pairs: number; needed: number; pct_per_usd: number | null }
}

export type Live = {
  model: string | null
  effort: string | null
  cache: Cache
  forecast: Forecast | null
  limits: LimitWindow[]
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

export type DaemonStatus = 'starting' | 'ok' | 'rebuilding' | 'unreachable' | 'outdated' | 'behind' | 'newer' | 'missing' | 'failed'

export type Daemon = { status: DaemonStatus; detail: string | null }

export type Motion = { from: number; to: number; startMs: number }

export type PaneState = { session: string | null; asked: boolean; unasked: 'waiting' | 'settled' }

declare module 'claude-code' {
  interface PluginState {
    plus: {
      view: PlusView
      snapshot: Shaped<Snapshot | null>
      context: Shaped<ContextReport | null>
      local: Shaped<Local>
      pane: PaneState
      daemon: Daemon
      now: number
      expanded: string[]
      motions: Record<string, Motion>
    }
  }
}
