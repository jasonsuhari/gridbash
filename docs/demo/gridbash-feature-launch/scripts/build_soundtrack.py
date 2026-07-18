"""Build the deterministic original soundtrack for the GridBash feature film."""

from __future__ import annotations

import wave
from pathlib import Path

import numpy as np


SAMPLE_RATE = 48_000
DURATION = 96.0
BPM = 112.0
BEAT = 60.0 / BPM
TAU = 2.0 * np.pi
OUTPUT = Path(__file__).resolve().parents[1] / "assets" / "feature-launch-bed.wav"

SCENE_STARTS = np.array((0.0, 6.0, 16.0, 27.0, 38.0, 50.0, 62.0, 76.0, 86.0))
SCENE_ROOTS = np.array((55.0, 55.0, 65.41, 61.74, 73.42, 65.41, 55.0, 61.74, 73.42))
ROUTE_TICKS = (7.2, 7.55, 7.9, 8.25, 52.0, 52.2, 64.1, 64.7, 65.1, 65.5, 78.0, 78.3, 78.6, 89.2)


def decaying_event(time_s: np.ndarray, event: float, length: float, curve: float) -> np.ndarray:
    local = time_s - event
    mask = (local >= 0.0) & (local <= length)
    result = np.zeros_like(time_s)
    result[mask] = np.exp(-curve * local[mask] / length)
    return result


def build() -> None:
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(244)
    frame_count = int(DURATION * SAMPLE_RATE)
    block_frames = SAMPLE_RATE * 2

    with wave.open(str(OUTPUT), "wb") as stream:
        stream.setnchannels(2)
        stream.setsampwidth(2)
        stream.setframerate(SAMPLE_RATE)

        for start in range(0, frame_count, block_frames):
            stop = min(start + block_frames, frame_count)
            time_s = np.arange(start, stop, dtype=np.float64) / SAMPLE_RATE
            roots = SCENE_ROOTS[np.searchsorted(SCENE_STARTS, time_s, side="right") - 1]
            beat_phase = np.mod(time_s, BEAT)
            half_phase = np.mod(time_s, BEAT / 2.0)

            pad_l = 0.012 * np.sin(TAU * roots * time_s)
            pad_l += 0.007 * np.sin(TAU * roots * 1.25 * time_s + 0.4)
            pad_r = 0.012 * np.sin(TAU * roots * time_s + 0.08)
            pad_r += 0.007 * np.sin(TAU * roots * 1.5 * time_s + 0.7)

            kick_env = np.where(beat_phase <= 0.22, np.exp(-6.6 * beat_phase / 0.22), 0.0)
            kick_frequency = 46.0 + 62.0 * kick_env
            kick = 0.11 * kick_env * np.sin(TAU * kick_frequency * beat_phase)

            tick_env = np.where(half_phase <= 0.065, np.exp(-8.0 * half_phase / 0.065), 0.0)
            tick = 0.018 * tick_env * rng.uniform(-1.0, 1.0, len(time_s))

            impact = np.zeros_like(time_s)
            whoosh = np.zeros_like(time_s)
            for event in SCENE_STARTS[1:]:
                impact_env = decaying_event(time_s, float(event), 0.72, 5.2)
                local = np.maximum(0.0, time_s - event)
                impact += 0.13 * impact_env * np.sin(TAU * (42.0 + 28.0 * impact_env) * local)
                pre = np.clip(1.0 - np.abs(time_s - event) / 0.04, 0.0, 1.0)
                whoosh += 0.035 * pre * rng.uniform(-1.0, 1.0, len(time_s))

            route = np.zeros_like(time_s)
            for event in ROUTE_TICKS:
                route_env = decaying_event(time_s, event, 0.12, 7.5)
                local = np.maximum(0.0, time_s - event)
                route += 0.045 * route_env * np.sin(TAU * 880.0 * local)

            master = 0.84
            left = master * (pad_l + kick + tick * 0.82 + impact + whoosh + route * 0.92)
            right = master * (pad_r + kick + tick + impact + whoosh * 0.84 + route)
            stereo = np.empty((len(time_s), 2), dtype=np.int16)
            stereo[:, 0] = (np.clip(left, -0.98, 0.98) * 32767).astype(np.int16)
            stereo[:, 1] = (np.clip(right, -0.98, 0.98) * 32767).astype(np.int16)
            stream.writeframesraw(stereo.tobytes())

    print(OUTPUT)


if __name__ == "__main__":
    build()
