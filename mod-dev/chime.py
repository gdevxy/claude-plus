import math
import struct
import sys
import wave
from pathlib import Path

RATE = 44_100
SECONDS = 1.0
PEAK = 0.32
CUTOFF = 2_800
FADE = 0.05
BLOOPS = ((0.0, 380.0, 640.0), (0.09, 520.0, 900.0))
GLIDE = 0.02
DECAY = 0.06
WET = 0.25
ROOM = 0.75
COMBS = (1557, 1617, 1491, 1422, 1277, 1356)
ALLPASSES = (556, 441)


def bloop(start: float, end: float) -> list[float]:
    samples, phase = [], 0.0
    for index in range(int(RATE * DECAY * 7)):
        t = index / RATE
        pitch = end + (start - end) * math.exp(-t / GLIDE)
        phase += 2 * math.pi * pitch / RATE
        envelope = (1 - math.exp(-t / 0.002)) * math.exp(-t / DECAY)
        samples.append(envelope * (math.sin(phase) + 0.15 * math.sin(2 * phase)))
    return samples


def mixed() -> list[float]:
    samples = [0.0] * int(RATE * SECONDS)
    for offset, start, end in BLOOPS:
        first = int(RATE * offset)
        for index, value in enumerate(bloop(start, end)):
            samples[first + index] += value
    return samples


def lowpass(samples: list[float]) -> list[float]:
    alpha = 1 - math.exp(-2 * math.pi * CUTOFF / RATE)
    out, level = [], 0.0
    for value in samples:
        level += alpha * (value - level)
        out.append(level)
    return out


def comb(samples: list[float], delay: int) -> list[float]:
    buffer, out, damped = [0.0] * delay, [], 0.0
    for index, value in enumerate(samples):
        echoed = buffer[index % delay]
        damped = 0.6 * damped + 0.4 * echoed
        buffer[index % delay] = value + ROOM * damped
        out.append(echoed)
    return out


def allpass(samples: list[float], delay: int) -> list[float]:
    buffer, out = [0.0] * delay, []
    for index, value in enumerate(samples):
        echoed = buffer[index % delay]
        buffer[index % delay] = value + 0.5 * echoed
        out.append(echoed - value)
    return out


def reverb(samples: list[float]) -> list[float]:
    combs = [comb(samples, delay) for delay in COMBS]
    tail = [sum(column) / len(COMBS) for column in zip(*combs)]
    for delay in ALLPASSES:
        tail = allpass(tail, delay)
    return [(1 - WET) * dry + WET * wet for dry, wet in zip(samples, tail)]


def finished(samples: list[float]) -> list[float]:
    loudest = max(abs(value) for value in samples)
    out = [value * PEAK / loudest for value in samples]
    fade = int(RATE * FADE)
    for index in range(fade):
        out[-1 - index] *= index / fade
    return out


def main() -> None:
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "mod" / "sounds" / "chime.wav"
    target.parent.mkdir(parents=True, exist_ok=True)
    samples = finished(reverb(lowpass(mixed())))
    frames = b"".join(struct.pack("<h", round(32_767 * value)) for value in samples)
    with wave.open(str(target), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(frames)


if __name__ == "__main__":
    main()
