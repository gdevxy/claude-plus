import type { ThemeKey } from 'claude-code'

export const DEFAULT_COLOR = 0x01000000

export type Palette = Readonly<Partial<Record<ThemeKey, number>>>

function rgb(red: number, green: number, blue: number): number {
  return (red << 16) | (green << 8) | blue
}

const DARK: Palette = {
  claude: rgb(215, 119, 87),
  permission: rgb(177, 185, 249),
  suggestion: rgb(177, 185, 249),
  remember: rgb(177, 185, 249),
  autoAccept: rgb(175, 135, 255),
  planMode: rgb(72, 150, 140),
  bashBorder: rgb(253, 93, 177),
  ide: rgb(71, 130, 200),
  promptBorder: rgb(136, 136, 136),
  text: rgb(255, 255, 255),
  inactive: rgb(153, 153, 153),
  subtle: rgb(80, 80, 80),
  success: rgb(78, 186, 101),
  error: rgb(255, 107, 128),
  warning: rgb(255, 193, 7),
}

const LIGHT: Palette = {
  claude: rgb(215, 119, 87),
  permission: rgb(87, 105, 247),
  suggestion: rgb(87, 105, 247),
  remember: rgb(0, 0, 255),
  autoAccept: rgb(135, 0, 255),
  planMode: rgb(0, 102, 102),
  bashBorder: rgb(255, 0, 135),
  ide: rgb(71, 130, 200),
  promptBorder: rgb(138, 138, 138),
  text: rgb(0, 0, 0),
  inactive: rgb(102, 102, 102),
  subtle: rgb(175, 175, 175),
  success: rgb(44, 122, 57),
  error: rgb(171, 43, 63),
  warning: rgb(150, 108, 30),
}

const LIGHT_DALTONIZED: Palette = {
  claude: rgb(255, 153, 51),
  permission: rgb(51, 102, 255),
  suggestion: rgb(51, 102, 255),
  remember: rgb(51, 102, 255),
  autoAccept: rgb(135, 0, 255),
  planMode: rgb(51, 102, 102),
  bashBorder: rgb(0, 102, 204),
  ide: rgb(71, 130, 200),
  promptBorder: rgb(138, 138, 138),
  text: rgb(0, 0, 0),
  inactive: rgb(102, 102, 102),
  subtle: rgb(175, 175, 175),
  success: rgb(0, 102, 153),
  error: rgb(204, 0, 0),
  warning: rgb(255, 153, 0),
}

const DARK_DALTONIZED: Palette = {
  claude: rgb(255, 153, 51),
  permission: rgb(153, 204, 255),
  suggestion: rgb(153, 204, 255),
  remember: rgb(153, 204, 255),
  autoAccept: rgb(175, 135, 255),
  planMode: rgb(102, 153, 153),
  bashBorder: rgb(51, 153, 255),
  ide: rgb(71, 130, 200),
  promptBorder: rgb(136, 136, 136),
  text: rgb(255, 255, 255),
  inactive: rgb(153, 153, 153),
  subtle: rgb(80, 80, 80),
  success: rgb(51, 153, 255),
  error: rgb(255, 102, 102),
  warning: rgb(255, 204, 0),
}

const PALETTES: Readonly<Record<string, Palette>> = {
  dark: DARK,
  light: LIGHT,
  'light-daltonized': LIGHT_DALTONIZED,
  'dark-daltonized': DARK_DALTONIZED,
}

export function paletteOf(theme: string): Palette | null {
  return PALETTES[theme] ?? null
}

export function colorOf(palette: Palette, key: ThemeKey | null): number {
  return key === null ? DEFAULT_COLOR : (palette[key] ?? DEFAULT_COLOR)
}

export type Series = { readonly models: readonly ThemeKey[]; readonly other: ThemeKey }

const DEFAULT_SERIES: Series = { models: ['claude', 'permission', 'planMode', 'ide', 'text'], other: 'subtle' }

const SERIES: Readonly<Record<string, Series>> = {
  dark: { models: ['claude', 'permission', 'ide', 'text', 'inactive'], other: 'subtle' },
  light: { models: ['claude', 'planMode', 'ide', 'text', 'remember'], other: 'subtle' },
  'light-daltonized': { models: ['claude', 'permission', 'planMode', 'text', 'success'], other: 'subtle' },
  'dark-daltonized': { models: ['claude', 'permission', 'ide', 'text', 'inactive'], other: 'subtle' },
}

export function seriesOf(theme: string): Series {
  return SERIES[theme] ?? DEFAULT_SERIES
}
