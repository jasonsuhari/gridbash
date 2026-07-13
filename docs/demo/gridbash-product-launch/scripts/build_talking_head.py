#!/usr/bin/env python3
"""Build clean GridBash talking-head takes, voice timeline, and captions."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
from dataclasses import asdict, dataclass
from pathlib import Path


FINAL_DURATION = 45.16


@dataclass(frozen=True)
class Take:
    key: str
    source_start: float
    source_end: float
    timeline_start: float

    @property
    def duration(self) -> float:
        return round(self.source_end - self.source_start, 3)

    @property
    def timeline_end(self) -> float:
        return round(self.timeline_start + self.duration, 3)


TAKES = (
    Take("01-hook", 2.28, 7.65, 0.00),
    Take("02-motivation", 99.98, 107.41, 6.32),
    Take("03-product", 14.32, 19.52, 13.75),
    Take("04-terminal", 30.76, 33.80, 20.04),
    Take("05-routing", 37.98, 44.56, 23.08),
    Take("06-worktrees", 49.74, 56.12, 29.75),
    Take("07-cta", 112.24, 120.12, 37.18),
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source",
        type=Path,
        default=Path("raw/jason-talking-head.mp4"),
        help="Original camera recording",
    )
    parser.add_argument(
        "--transcript",
        type=Path,
        default=Path("transcripts/jason-talking-head.json"),
        help="Whisper JSON with word timestamps",
    )
    parser.add_argument("--skip-video", action="store_true", help="Only rebuild voice and caption data")
    return parser.parse_args()


def require_tool(name: str) -> None:
    if shutil.which(name) is None:
        raise SystemExit(f"Required executable is not on PATH: {name}")


def run(command: list[str]) -> None:
    subprocess.run(command, check=True)


def build_video_takes(source: Path, output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    video_filter = (
        "scale=1920:1080:flags=lanczos,"
        "unsharp=5:5:0.35:5:5:0.0,"
        "setsar=1"
    )
    for take in TAKES:
        output = output_dir / f"{take.key}.mp4"
        run(
            [
                "ffmpeg",
                "-v",
                "error",
                "-y",
                "-ss",
                f"{take.source_start:.3f}",
                "-t",
                f"{take.duration:.3f}",
                "-i",
                str(source),
                "-an",
                "-vf",
                video_filter,
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "10",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                str(output),
            ]
        )
        print(f"Built {output} ({take.duration:.2f}s)")


def build_voice(source: Path, output: Path) -> None:
    filters: list[str] = []
    concat_inputs: list[str] = []
    cursor = 0.0
    audio_index = 0

    for index, take in enumerate(TAKES):
        gap = round(take.timeline_start - cursor, 3)
        if gap > 0.001:
            label = f"gap{index}"
            filters.append(f"anullsrc=r=48000:cl=stereo:d={gap:.3f}[{label}]")
            concat_inputs.append(f"[{label}]")
            audio_index += 1

        label = f"take{index}"
        filters.append(
            f"[0:a]atrim=start={take.source_start:.3f}:end={take.source_end:.3f},"
            f"asetpts=PTS-STARTPTS,aformat=sample_rates=48000:channel_layouts=stereo[{label}]"
        )
        concat_inputs.append(f"[{label}]")
        audio_index += 1
        cursor = take.timeline_end

    final_gap = round(FINAL_DURATION - cursor, 3)
    if final_gap > 0.001:
        filters.append(f"anullsrc=r=48000:cl=stereo:d={final_gap:.3f}[tail]")
        concat_inputs.append("[tail]")
        audio_index += 1

    filters.append(
        "".join(concat_inputs)
        + f"concat=n={audio_index}:v=0:a=1[voice];"
        + f"[voice]loudnorm=I=-16:TP=-1.5:LRA=11,aresample=48000,"
        + f"apad=pad_dur=0.25,atrim=duration={FINAL_DURATION:.3f}[out]"
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "ffmpeg",
            "-v",
            "error",
            "-y",
            "-i",
            str(source),
            "-filter_complex",
            ";".join(filters),
            "-map",
            "[out]",
            "-t",
            f"{FINAL_DURATION:.3f}",
            "-c:a",
            "pcm_s24le",
            str(output),
        ]
    )
    print(f"Built {output} ({FINAL_DURATION:.2f}s)")


def corrected_word(text: str, take: Take) -> str:
    stripped = text.strip()
    if take.key == "01-hook" and stripped.lower() == "cloud":
        return "Claude"
    return stripped


def build_captions(transcript: Path, output: Path) -> None:
    data = json.loads(transcript.read_text(encoding="utf-8"))
    source_words = [word for segment in data.get("segments", []) for word in segment.get("words", [])]
    mapped_words: list[dict] = []
    mapped_takes: list[dict] = []

    for take in TAKES:
        take_word_indexes: list[int] = []
        for word in source_words:
            word_start = float(word["start"])
            word_end = float(word["end"])
            if word_end <= take.source_start or word_start >= take.source_end:
                continue
            mapped = {
                "text": corrected_word(str(word["word"]), take),
                "start": round(take.timeline_start + word_start - take.source_start, 3),
                "end": round(take.timeline_start + min(word_end, take.source_end) - take.source_start, 3),
                "take": take.key,
            }
            if take_word_indexes:
                previous = mapped_words[take_word_indexes[-1]]
                current_text = mapped["text"]
                pair = (previous["text"].lower(), current_text.lower())
                if current_text.startswith("-"):
                    previous["text"] += current_text
                    previous["end"] = mapped["end"]
                    continue
                if pair == ("grid", "bash"):
                    previous["text"] = "GridBash"
                    previous["end"] = mapped["end"]
                    continue
                if pair == ("work", "tree"):
                    previous["text"] = "worktree"
                    previous["end"] = mapped["end"]
                    continue
            take_word_indexes.append(len(mapped_words))
            mapped_words.append(mapped)
        mapped_takes.append({**asdict(take), "duration": take.duration, "timeline_end": take.timeline_end, "word_indexes": take_word_indexes})

    groups: list[dict] = []
    for take_info in mapped_takes:
        indexes = take_info["word_indexes"]
        current: list[int] = []
        for index in indexes:
            word = mapped_words[index]
            if current:
                previous = mapped_words[current[-1]]
                pause = word["start"] - previous["end"]
                if len(current) >= 4 or pause >= 0.18 or previous["text"].endswith((".", "?", "!", ",")):
                    groups.append(make_group(current, mapped_words))
                    current = []
            current.append(index)
        if current:
            groups.append(make_group(current, mapped_words))

    payload = {
        "duration": FINAL_DURATION,
        "takes": mapped_takes,
        "words": mapped_words,
        "groups": groups,
    }
    output.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    output.with_suffix(".js").write_text(
        "window.GRIDBASH_CAPTIONS = " + json.dumps(payload, separators=(",", ":")) + ";\n",
        encoding="utf-8",
    )
    print(f"Built {output} ({len(mapped_words)} words, {len(groups)} groups)")


def make_group(indexes: list[int], words: list[dict]) -> dict:
    group_words = [words[index] for index in indexes]
    return {
        "start": group_words[0]["start"],
        "end": group_words[-1]["end"],
        "take": group_words[0]["take"],
        "word_indexes": indexes,
        "text": " ".join(word["text"] for word in group_words),
    }


def main() -> int:
    args = parse_args()
    require_tool("ffmpeg")
    source = args.source.resolve()
    transcript = args.transcript.resolve()
    if not source.is_file():
        raise SystemExit(f"Missing source video: {source}")
    if not transcript.is_file():
        raise SystemExit(f"Missing transcript JSON: {transcript}")

    if not args.skip_video:
        build_video_takes(source, Path("assets/takes"))
    build_voice(source, Path("assets/voice.wav"))
    build_captions(transcript, Path("assets/captions.json"))

    manifest = {
        "source": Path(os.path.relpath(source, Path.cwd())).as_posix(),
        "transcript": Path(os.path.relpath(transcript, Path.cwd())).as_posix(),
        "duration": FINAL_DURATION,
        "takes": [{**asdict(take), "duration": take.duration, "timeline_end": take.timeline_end} for take in TAKES],
    }
    Path("edit-manifest.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
