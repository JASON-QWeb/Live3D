"""Measure decoded frame timing in a real local OBS or cloud recording."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from fractions import Fraction
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile


def number(value: str | None) -> float | None:
    if not value or value == "N/A":
        return None
    result = float(value)
    return result if math.isfinite(result) else None


def nominal_rate(stream: dict) -> float | None:
    for name in ("avg_frame_rate", "r_frame_rate"):
        value = stream.get(name)
        if value and value not in ("N/A", "0/0"):
            rate = float(Fraction(value))
            if rate > 0:
                return rate
    return None


def rounded(value: float | None, scale: float = 1) -> float | None:
    return round(value * scale, 4) if value is not None else None


@dataclass
class Track:
    stream: dict
    fps: float | None
    decoded: int = 0
    timed: int = 0
    first: float | None = None
    last: float | None = None
    end: float | None = None
    previous: float | None = None
    previous_end: float | None = None
    last_has_length: bool = False
    max_interval: float | None = None
    max_gap: float | None = None
    long_intervals: int = 0
    backward: int = 0

    def observe(self, frame: dict[str, str]) -> None:
        self.decoded += 1
        stamp = number(frame.get("best_effort_timestamp_time"))
        if stamp is None:
            return
        self.timed += 1
        length = number(frame.get("duration_time"))
        samples = number(frame.get("nb_samples"))
        sample_rate = int(self.stream.get("sample_rate") or 0)
        if samples is not None and sample_rate > 0:
            length = samples / sample_rate
        if length is not None and length <= 0:
            length = None
        if self.previous is not None:
            step = stamp - self.previous
            self.backward += int(step < -0.000001)
            self.max_interval = max(self.max_interval or 0.0, step)
            if self.fps and step > 1.5 / self.fps:
                self.long_intervals += 1
        if self.previous_end is not None:
            self.max_gap = max(self.max_gap or 0.0, stamp - self.previous_end)
        self.first = stamp if self.first is None else min(self.first, stamp)
        if self.last is None or stamp >= self.last:
            self.last = stamp
            self.last_has_length = length is not None
        self.previous = stamp
        self.previous_end = stamp + length if length is not None else None
        if self.previous_end is not None:
            self.end = (
                max(self.end, self.previous_end) if self.end is not None else self.previous_end
            )

    @property
    def known_end(self) -> float | None:
        return self.end if self.last_has_length else None

    def report(self, video: Track | None) -> dict:
        out = {
            k: self.stream[k]
            for k in ("index", "codec_type", "codec_name", "width", "height", "sample_rate")
            if k in self.stream
        }
        out.update(
            decoded_frames=self.decoded,
            frames_with_pts=self.timed,
            start_s=rounded(self.first),
            end_s=rounded(self.known_end),
            backward_timestamps=self.backward,
        )
        if self.stream["codec_type"] == "video":
            span = (
                self.last - self.first if self.last is not None and self.first is not None else 0.0
            )
            out.update(
                mean_fps=rounded((self.timed - 1) / span if span > 0 else None),
                nominal_fps=rounded(self.fps),
                max_frame_interval_ms=rounded(self.max_interval, 1000),
                intervals_over_1_5_nominal_frames=self.long_intervals if self.fps else None,
            )
        else:
            out["max_audio_frame_gap_ms"] = rounded(self.max_gap, 1000)
            if video and self.first is not None and video.first is not None:
                out["start_minus_video_ms"] = rounded(self.first - video.first, 1000)
            if video and self.known_end is not None and video.known_end is not None:
                out["end_minus_video_ms"] = rounded(self.known_end - video.known_end, 1000)
        return out


def inspect(path: Path) -> dict:
    base = ["ffprobe", "-v", "error", "-protocol_whitelist", "file,pipe"]
    metadata = subprocess.run(
        [*base, "-show_streams", "-of", "json", "-i", str(path)], check=True, stdout=subprocess.PIPE
    )
    streams = json.loads(metadata.stdout)["streams"]
    tracks = {
        s["index"]: Track(s, nominal_rate(s) if s["codec_type"] == "video" else None)
        for s in streams
        if s["codec_type"] in ("video", "audio")
        and not s.get("disposition", {}).get("attached_pic")
    }
    if not tracks:
        raise ValueError("文件中没有可检查的视频或音频轨")
    fields = "frame=stream_index,best_effort_timestamp_time,duration_time,nb_samples"
    cmd = [
        *base,
        "-threads",
        "2",
        "-show_frames",
        "-show_entries",
        fields,
        "-of",
        "compact=p=0:nk=0",
        "-i",
        str(path),
    ]
    with (
        tempfile.TemporaryFile(mode="w+") as errors,
        subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=errors, text=True) as probe,
    ):
        for line in probe.stdout:
            frame = dict(part.split("=", 1) for part in line.strip().split("|") if "=" in part)
            if (index := frame.get("stream_index")) is not None and int(index) in tracks:
                tracks[int(index)].observe(frame)
        code = probe.wait()
        errors.seek(0)
        error = errors.read(500).strip()
        if code or error:
            raise ValueError(error or f"ffprobe 退出码 {code}")
    if not any(track.timed for track in tracks.values()):
        raise ValueError("没有可核对时间戳的音视频帧")
    videos = [track for track in tracks.values() if track.stream["codec_type"] == "video"]
    reference = next(
        (v for v in videos if v.stream.get("disposition", {}).get("default")),
        next(iter(videos), None),
    )
    return {
        "file": path.name,
        "reference_video_index": reference.stream["index"] if reference else None,
        "tracks": [track.report(reference) for track in tracks.values()],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(
        description="核对本机录制文件的帧率、时间间隔及音视频轨起止时间"
    )
    parser.add_argument("recording", type=Path, help="实际录制的本地文件")
    args = parser.parse_args()
    if not args.recording.is_file():
        parser.error("录制文件不存在")
    try:
        print(json.dumps(inspect(args.recording.resolve()), ensure_ascii=False, indent=2))
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        sys.exit(f"无法检查录制文件：{exc}")
