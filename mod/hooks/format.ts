const MINUTE_MS = 60_000
const HOUR_MS = 3_600_000
const DAY_MS = 86_400_000
const EIGHTHS = ['', '▏', '▎', '▍', '▌', '▋', '▊', '▉']
const FAMILIES = ['opus', 'sonnet', 'haiku', 'fable', 'mythos']
const TOKEN_UNITS = ['k', 'M', 'B', 'T']
const WINDOWS: Record<string, string> = { five_hour: '5h', seven_day: '7d', spend_limit: 'Spend' }

export const FULL_BLOCK = '█'
export const TRACK = '░'
export const ELLIPSIS = '…'

export function money(usd: number): string {
  if (usd === 0) return '$0.00'
  if (usd < 0.01) return '<$0.01'
  if (usd < 1000) return `$${usd.toFixed(2)}`
  return `$${grouped(Math.round(usd))}`
}

export function approx(usd: number): string {
  return usd > 0 && usd < 0.01 ? money(usd) : `~${money(usd)}`
}

export function count(value: number): string {
  return grouped(Math.round(value))
}

export function tokens(count: number): string {
  if (count < 1000) return String(Math.round(count))
  return scaled(count / 1000, 0)
}

export function percent(value: number): string {
  if (value > 0 && value < 1) return '<1%'
  return `${Math.round(value)}%`
}

export function finePercent(value: number): string {
  if (value > 0 && value < 0.1) return '<0.1%'
  return value < 10 ? `${value.toFixed(1)}%` : `${Math.round(value)}%`
}

export function countdown(ms: number): string {
  const seconds = Math.max(0, Math.ceil(ms / 1000))
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const rest = String(seconds % 60).padStart(2, '0')
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${rest}` : `${minutes}:${rest}`
}

export function duration(ms: number): string {
  if (ms < MINUTE_MS) return '<1m'
  if (ms < HOUR_MS) return `${Math.floor(ms / MINUTE_MS)}m`
  if (ms < DAY_MS) {
    const minutes = Math.floor((ms % HOUR_MS) / MINUTE_MS)
    return minutes === 0 ? `${Math.floor(ms / HOUR_MS)}h` : `${Math.floor(ms / HOUR_MS)}h ${minutes}m`
  }
  const hours = Math.floor((ms % DAY_MS) / HOUR_MS)
  return hours === 0 ? `${Math.floor(ms / DAY_MS)}d` : `${Math.floor(ms / DAY_MS)}d ${hours}h`
}

export function modelName(id: string): string {
  const parts = id.replace(/\[.*\]$/, '').replace(/-\d{8}$/, '').split('-')
  const at = parts.findIndex(part => FAMILIES.includes(part))
  if (at < 0) return id
  const family = parts[at] ?? ''
  const version = parts.slice(at + 1).filter(part => /^\d+$/.test(part))
  const name = family.charAt(0).toUpperCase() + family.slice(1)
  return version.length > 0 ? `${name} ${version.join('.')}` : name
}

export function windowName(kind: string): string {
  return WINDOWS[kind] ?? kind
}

export function fit(text: string, width: number): string {
  if (text.length <= width) return text
  return width <= 1 ? ELLIPSIS.slice(0, width) : `${text.slice(0, width - 1)}${ELLIPSIS}`
}

export function spread(left: string, right: string, width: number): string {
  const room = width - right.length - 1
  const shown = fit(left, Math.max(0, room))
  return `${shown}${' '.repeat(Math.max(1, width - shown.length - right.length))}${right}`
}

export type BarCells = { filled: string; track: string }

export function bar(fraction: number, width: number): BarCells {
  const clamped = Math.min(1, Math.max(0, Number.isFinite(fraction) ? fraction : 0))
  const eighths = Math.round(clamped * width * 8)
  const full = Math.floor(eighths / 8)
  const partial = EIGHTHS[eighths % 8] ?? ''
  const filled = FULL_BLOCK.repeat(full) + partial
  return { filled, track: TRACK.repeat(width - filled.length) }
}

function scaled(value: number, step: number): string {
  const rounded = value < 100 ? Math.round(value * 10) / 10 : Math.round(value)
  if (rounded >= 1000 && step < TOKEN_UNITS.length - 1) return scaled(value / 1000, step + 1)
  return `${String(rounded)}${TOKEN_UNITS[step] ?? ''}`
}

function grouped(value: number): string {
  return String(value).replace(/\B(?=(\d{3})+(?!\d))/g, ',')
}
