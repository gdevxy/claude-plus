import math
import struct
import sys
import wave
from pathlib import Path

RATE = 22_050
SECONDS = 0.6
PEAK = 0.3
ATTACK = 0.01
DECAY = 0.12
PARTIALS = ((880.0, 1.0), (1760.0, 0.3), (2640.0, 0.1))


def sample(index: int) -> int:
    t = index / RATE
    envelope = min(1.0, t / ATTACK) * math.exp(-t / DECAY)
    tone = sum(level * math.sin(2 * math.pi * pitch * t) for pitch, level in PARTIALS)
    loudest = sum(level for _, level in PARTIALS)
    return round(32_767 * PEAK * envelope * tone / loudest)


def main() -> None:
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent / "mod" / "sounds" / "chime.wav"
    target.parent.mkdir(parents=True, exist_ok=True)
    frames = b"".join(struct.pack("<h", sample(index)) for index in range(int(RATE * SECONDS)))
    with wave.open(str(target), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(frames)


if __name__ == "__main__":
    main()
