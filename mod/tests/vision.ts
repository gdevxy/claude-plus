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

const POW25_7 = 25 ** 7
const RAD = Math.PI / 180

function hueOf(b: number, a: number): number {
  const degrees = Math.atan2(b, a) / RAD
  return degrees < 0 ? degrees + 360 : degrees
}

export function distance(first: number, second: number, matrix: Matrix | null): number {
  const [l1, a1, b1] = lab(seen(first, matrix))
  const [l2, a2, b2] = lab(seen(second, matrix))
  const meanChroma = (Math.hypot(a1, b1) + Math.hypot(a2, b2)) / 2
  const g = 0.5 * (1 - Math.sqrt(meanChroma ** 7 / (meanChroma ** 7 + POW25_7)))
  const p1 = a1 * (1 + g)
  const p2 = a2 * (1 + g)
  const c1 = Math.hypot(p1, b1)
  const c2 = Math.hypot(p2, b2)
  const h1 = c1 === 0 ? 0 : hueOf(b1, p1)
  const h2 = c2 === 0 ? 0 : hueOf(b2, p2)
  const dl = l2 - l1
  const dc = c2 - c1
  let dh = h2 - h1
  if (dh > 180) dh -= 360
  if (dh < -180) dh += 360
  if (c1 * c2 === 0) dh = 0
  const dH = 2 * Math.sqrt(c1 * c2) * Math.sin((dh * RAD) / 2)
  const meanL = (l1 + l2) / 2
  const meanC = (c1 + c2) / 2
  let meanH = h1 + h2
  if (c1 * c2 !== 0) {
    if (Math.abs(h1 - h2) <= 180) meanH /= 2
    else meanH = (meanH + (h1 + h2 < 360 ? 360 : -360)) / 2
  }
  const t = 1 - 0.17 * Math.cos((meanH - 30) * RAD) + 0.24 * Math.cos(2 * meanH * RAD) + 0.32 * Math.cos((3 * meanH + 6) * RAD) - 0.2 * Math.cos((4 * meanH - 63) * RAD)
  const sl = 1 + (0.015 * (meanL - 50) ** 2) / Math.sqrt(20 + (meanL - 50) ** 2)
  const sc = 1 + 0.045 * meanC
  const sh = 1 + 0.015 * meanC * t
  const rc = 2 * Math.sqrt(meanC ** 7 / (meanC ** 7 + POW25_7))
  const rt = -Math.sin(2 * 30 * Math.exp(-(((meanH - 275) / 25) ** 2)) * RAD) * rc
  return Math.sqrt((dl / sl) ** 2 + (dc / sc) ** 2 + (dH / sh) ** 2 + rt * (dc / sc) * (dH / sh))
}
