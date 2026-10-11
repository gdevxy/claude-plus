type Matrix = readonly [readonly [number, number, number], readonly [number, number, number], readonly [number, number, number]]

export const VISIONS: Readonly<Record<string, Matrix | null>> = {
  normal: null,
  deuteranopia: [
    [0.367322, 0.860646, -0.227968],
    [0.280085, 0.672501, 0.047413],
    [-0.01182, 0.04294, 0.968881],
  ],
  protanopia: [
    [0.152286, 1.052583, -0.204868],
    [0.114503, 0.786281, 0.099216],
    [-0.003882, -0.048116, 1.051998],
  ],
}

type Rgb = readonly [number, number, number]

function linear(channel: number): number {
  const value = channel / 255
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
}

function channelsOf(color: number): Rgb {
  return [linear((color >> 16) & 0xff), linear((color >> 8) & 0xff), linear(color & 0xff)]
}

function seen(color: number, matrix: Matrix | null): Rgb {
  const rgb = channelsOf(color)
  if (matrix === null) return rgb
  const [r, g, b] = rgb
  return matrix.map(row => Math.min(1, Math.max(0, row[0] * r + row[1] * g + row[2] * b))) as unknown as Rgb
}

function lab([r, g, b]: Rgb): Rgb {
  const x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047
  const y = 0.2126 * r + 0.7152 * g + 0.0722 * b
  const z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883
  const f = (t: number) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116)
  return [116 * f(y) - 16, 500 * (f(x) - f(y)), 200 * (f(y) - f(z))]
}

export function distance(first: number, second: number, matrix: Matrix | null): number {
  const [l1, a1, b1] = lab(seen(first, matrix))
  const [l2, a2, b2] = lab(seen(second, matrix))
  return Math.hypot(l1 - l2, a1 - a2, b1 - b2)
}
