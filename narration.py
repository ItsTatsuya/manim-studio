"""Kokoro-82M narration: verified model files, timeline, synthesis and muxing.

Kokoro runs on the CPU through ONNX Runtime, so no GPU or PyTorch is needed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import sys
import threading
import urllib.request
import uuid

RELEASE = (
    "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/"
)
FILES = {
    "voices": {
        "name": "voices-v1.0.bin",
        "size": 28_214_398,
        "sha256": "bca610b8308e8d99f32e6fe4197e7ec01679264efed0cac9140fe9c29f1fbf7d",
    },
    "standard": {
        "name": "kokoro-v1.0.int8.onnx",
        "size": 92_361_271,
        "sha256": "6e742170d309016e5891a994e1ce1559c702a2ccd0075e67ef7157974f6406cb",
    },
    "high": {
        "name": "kokoro-v1.0.onnx",
        "size": 325_532_387,
        "sha256": "7d5df8ecf7d4b1878015a32686053fd0eebe2bc377234608764cc0ef3636a6c5",
    },
}
SAMPLE_RATE = 24_000
OUTPUT_RATE = 48_000
# Pauses a narrator takes: between paragraphs, after sentences and after clauses.
GAP = 0.55
SENTENCE_PAUSE = 0.34
CLAUSE_PAUSE = 0.12
FADE = 0.008
# Web video players normalize around -14 to -16 LUFS; quieter narration plays
# noticeably softer than the music and speech around it.
TARGET_LUFS = -16.0
CEILING = 10 ** (-1.5 / 20)  # Sample-peak ceiling, about -1.5 dBFS.
# How much of the synthetic room response is mixed under the dry voice.
ROOM_MIX = 0.07
LANGUAGES = {
    "a": ("en-us", "American English"),
    "b": ("en-gb", "British English"),
    "e": ("es", "Spanish"),
    "f": ("fr-fr", "French"),
    "h": ("hi", "Hindi"),
    "i": ("it", "Italian"),
    "p": ("pt-br", "Brazilian Portuguese"),
}
VOICE_NAMES = (
    "af_heart af_bella af_nicole af_aoede af_kore af_sarah af_nova af_sky af_alloy "
    "af_jessica af_river am_michael am_fenrir am_puck am_echo am_eric am_liam am_onyx "
    "am_adam am_santa bf_emma bf_isabella bf_alice bf_lily bm_george bm_fable bm_lewis "
    "bm_daniel ef_dora em_alex em_santa ff_siwis hf_alpha hf_beta hm_omega hm_psi "
    "if_sara im_nicola pf_dora pm_alex pm_santa"
).split()
TIMESTAMP = re.compile(r"^\s*\[\s*(\d+(?::\d{1,2}){0,2}(?:\.\d+)?)\s*s?\s*\]\s*")
MAX_SEGMENTS = 400
# A timestamp or total narration beyond this would allocate gigabytes of audio.
MAX_SECONDS = 3600.0
# Kokoro reads at most 510 phonemes per pass. Longer paragraphs are cut into
# spoken-sentence chunks well below that, joined with natural pauses.
CHUNK_CHARS = 280
CACHE_VERSION = "1"
LINE_CACHE_LIMIT = 400 * 1024 * 1024
VIDEO_CACHE_KEEP = 3
VIDEO_CACHE_MAX_BYTES = 2 * 1024**3


class NarrationError(Exception):
    """A user-facing narration problem."""


def voices() -> list[dict]:
    result = []
    for voice in VOICE_NAMES:
        language, label = LANGUAGES[voice[0]]
        result.append(
            {
                "id": voice,
                "name": voice[3:].title(),
                "language": label,
                "gender": "Female" if voice[1] == "f" else "Male",
                "lang": language,
            }
        )
    return result


def language(voice: str) -> str:
    if voice not in VOICE_NAMES:
        raise NarrationError("Choose one of the listed narration voices.")
    return LANGUAGES[voice[0]][0]


def model_dir(data_root: Path) -> Path:
    return data_root / ".studio" / "models" / "kokoro"


def required_files(quality: str) -> list[dict]:
    if quality not in {"standard", "high"}:
        raise NarrationError("Choose Standard or High voice quality.")
    return [FILES["voices"], FILES[quality]]


def installed(data_root: Path, quality: str) -> bool:
    directory = model_dir(data_root)
    return all(
        (directory / item["name"]).is_file()
        and (directory / item["name"]).stat().st_size == item["size"]
        for item in required_files(quality)
    )


def download_size(data_root: Path, quality: str) -> int:
    directory = model_dir(data_root)
    return sum(
        item["size"]
        for item in required_files(quality)
        if not (directory / item["name"]).is_file()
        or (directory / item["name"]).stat().st_size != item["size"]
    )


def parse_timestamp(value: str) -> float:
    seconds = 0.0
    for part in value.split(":"):
        seconds = seconds * 60 + float(part)
    return seconds


def format_time(seconds: float) -> str:
    minutes, remainder = divmod(max(0.0, seconds), 60)
    return f"{int(minutes)}:{remainder:04.1f}"


def parse_timeline(text: str) -> list[dict]:
    """Paragraphs become narration lines; a [m:ss] prefix pins a start time."""
    segments, current = [], None
    for number, raw in enumerate(text.replace("\r\n", "\n").split("\n"), 1):
        line = raw.strip()
        if line.startswith("#"):
            continue
        if not line:
            current = None
            continue
        match = TIMESTAMP.match(line)
        if match and parse_timestamp(match[1]) > MAX_SECONDS:
            raise NarrationError(
                f"Narration line {number} starts after {int(MAX_SECONDS // 60)} minutes; "
                "use a time within the first hour."
            )
        if match or current is None:
            current = {
                "start": parse_timestamp(match[1]) if match else None,
                "text": "",
                "line": number,
            }
            segments.append(current)
            line = line[match.end() :] if match else line
        current["text"] = f"{current['text']} {line}".strip()
    segments = [segment for segment in segments if segment["text"]]
    if len(segments) > MAX_SEGMENTS:
        raise NarrationError(
            f"Narration has {len(segments)} lines; keep it under {MAX_SEGMENTS}."
        )
    return segments


class ModelDownloader:
    """One verified download at a time, resumable by simply retrying."""

    def __init__(self, data_root: Path):
        self.data_root = data_root
        self.lock = threading.Lock()
        self.state = {"status": "idle", "quality": None, "done": 0, "total": 0}
        self.thread = None
        self.cancel_event = threading.Event()

    def snapshot(self) -> dict:
        with self.lock:
            return dict(self.state)

    def start(self, quality: str):
        required_files(quality)
        with self.lock:
            if self.state["status"] == "downloading":
                raise NarrationError("A voice download is already running.")
            self.cancel_event.clear()
            self.state = {
                "status": "downloading",
                "quality": quality,
                "done": 0,
                "total": download_size(self.data_root, quality),
            }
            self.thread = threading.Thread(
                target=self._run, args=(quality,), daemon=True
            )
            self.thread.start()
        return self.snapshot()

    def cancel(self):
        self.cancel_event.set()
        return self.snapshot()

    def shutdown(self):
        self.cancel_event.set()
        thread = self.thread
        if thread is not None:
            thread.join(timeout=10)

    def _progress(self, amount: int):
        with self.lock:
            self.state["done"] += amount

    def _run(self, quality: str):
        try:
            directory = model_dir(self.data_root)
            directory.mkdir(parents=True, exist_ok=True)
            for item in required_files(quality):
                target = directory / item["name"]
                if target.is_file() and target.stat().st_size == item["size"]:
                    continue
                self._fetch(item, target)
            with self.lock:
                self.state.update(status="ready", error=None)
        except Exception as error:  # Network, disk and verification failures.
            with self.lock:
                cancelled = self.cancel_event.is_set()
                self.state.update(
                    status="cancelled" if cancelled else "failed",
                    error=None
                    if cancelled
                    else f"Couldn’t download the voice model: {error}",
                )

    def _fetch(self, item: dict, target: Path):
        partial = target.with_name(target.name + ".part")
        digest = hashlib.sha256()
        request = urllib.request.Request(
            RELEASE + item["name"], headers={"User-Agent": "ManimStudio"}
        )
        try:
            with (
                urllib.request.urlopen(request, timeout=60) as response,
                partial.open("wb") as output,
            ):
                while chunk := response.read(1 << 20):
                    if self.cancel_event.is_set():
                        raise NarrationError("Download cancelled.")
                    output.write(chunk)
                    digest.update(chunk)
                    self._progress(len(chunk))
            if (
                partial.stat().st_size != item["size"]
                or digest.hexdigest() != item["sha256"]
            ):
                raise NarrationError(
                    f"{item['name']} failed verification. Try the download again."
                )
            partial.replace(target)
        finally:
            partial.unlink(missing_ok=True)


def load_engine(data_root: Path, quality: str):
    if not installed(data_root, quality):
        raise NarrationError(
            "Download the narration voice in the Narration tab before rendering."
        )
    from kokoro_onnx import Kokoro

    directory = model_dir(data_root)
    return Kokoro(
        str(directory / FILES[quality]["name"]),
        str(directory / FILES["voices"]["name"]),
    )


_SPOKEN_SYMBOLS = {
    "≈": " approximately ",
    "≠": " is not equal to ",
    "≤": " is less than or equal to ",
    "≥": " is greater than or equal to ",
    "±": " plus or minus ",
    "×": " times ",
    "÷": " divided by ",
    "→": " to ",
    "⇒": " implies ",
    "∞": " infinity ",
    "∑": " the sum of ",
    "∫": " the integral of ",
    "∂": " partial ",
    "Δ": " delta ",
    "π": " pi ",
    "θ": " theta ",
    "α": " alpha ",
    "β": " beta ",
    "γ": " gamma ",
    "λ": " lambda ",
    "μ": " mu ",
    "σ": " sigma ",
    "φ": " phi ",
    "ω": " omega ",
    "°": " degrees ",
    "&": " and ",
    "²": " squared ",
    "³": " cubed ",
}
_ABBREVIATIONS = [
    (re.compile(r"\be\.g\.,?", re.I), "for example,"),
    (re.compile(r"\bi\.e\.,?", re.I), "that is,"),
    (re.compile(r"\betc\.", re.I), "et cetera."),
    (re.compile(r"\bvs\.?(?=\s)", re.I), "versus"),
    (re.compile(r"\bapprox\.", re.I), "approximately"),
]
_FRACTIONS = {
    "1/2": "one half",
    "1/3": "one third",
    "2/3": "two thirds",
    "1/4": "one quarter",
    "3/4": "three quarters",
}


def prepare_text(text: str) -> str:
    """Rewrite symbols and notation the way a narrator would say them."""
    text = re.sub(r"https?://\S+", "", text)
    text = re.sub(r"[*_`]+", "", text)  # Markdown emphasis is not spoken.
    for pattern, spoken in _ABBREVIATIONS:
        text = pattern.sub(spoken, text)
    text = re.sub(r"\s+(?:—|–|--)\s+|—", ", ", text)
    text = re.sub(r"(\w)\s*\^\s*2\b", r"\1 squared", text)
    text = re.sub(r"(\w)\s*\^\s*3\b", r"\1 cubed", text)
    text = re.sub(r"(\w)\s*\^\s*\(?(-?\w+)\)?", r"\1 to the \2", text)
    text = re.sub(r"\b([a-zA-Z])\(([a-zA-Z])\)", r"\1 of \2", text)
    text = re.sub(r"\s*\(([^()]{1,200})\)(?!\w)", r", \1,", text)
    text = re.sub(r"√\s*(\w+)", r" the square root of \1", text)
    text = re.sub(r"→\s*∞", " approaches infinity", text)
    text = re.sub(
        r"(?<![\d/])(\d)/(\d)(?![\d/])",
        lambda match: _FRACTIONS.get(match[0], f"{match[1]} over {match[2]}"),
        text,
    )
    for symbol, spoken in _SPOKEN_SYMBOLS.items():
        text = text.replace(symbol, spoken)
    text = re.sub(r"(?<=[\w)])\s*=\s*(?=[\w(-])", " equals ", text)
    text = re.sub(r"(?<=\w)\s+\+\s+(?=\w)", " plus ", text)
    text = re.sub(r"(?<=\w)\s+-\s+(?=\w)", " minus ", text)
    text = re.sub(r"(?<=\w)\s+<\s+(?=\w)", " is less than ", text)
    text = re.sub(r"(?<=\w)\s+>\s+(?=\w)", " is greater than ", text)
    text = re.sub(r"\.{3,}", "…", text)
    text = re.sub(r"\s+([,.;:!?…])", r"\1", text)
    text = re.sub(r",(?:\s*,)+", ",", text)
    text = re.sub(r"([.!?…;:]),", r"\1", text)
    return re.sub(r"\s+", " ", text).strip(" ,")


def _shape(samples):
    """Trim edge silence and fade both ends so lines never click."""
    import numpy as np

    if not samples.size:
        return samples
    level = np.abs(samples)
    loud = np.flatnonzero(level > max(float(level.max()) * 0.01, 1e-4))
    if loud.size:
        start = max(0, loud[0] - int(0.02 * SAMPLE_RATE))
        end = min(samples.size, loud[-1] + int(0.06 * SAMPLE_RATE))
        samples = samples[start:end]
    fade = min(int(FADE * SAMPLE_RATE), samples.size // 2)
    if fade:
        ramp = np.linspace(0.0, 1.0, fade, dtype=np.float32)
        samples = samples.copy()
        samples[:fade] *= ramp
        samples[-fade:] *= ramp[::-1]
    return samples


def _room(seconds=0.16):
    """A short, diffuse, deterministic room response without discrete echoes."""
    import numpy as np

    count = int(seconds * SAMPLE_RATE)
    noise = np.random.default_rng(82).standard_normal(count)
    response = noise * np.exp(-np.arange(count) / (0.035 * SAMPLE_RATE))
    response[: int(0.004 * SAMPLE_RATE)] = 0  # Pre-delay keeps the voice clear.
    # Soften the tail: a reflected voice loses its highs first.
    response = np.convolve(response, np.ones(6) / 6, mode="same")
    return response / np.sqrt(np.sum(response**2))


def _k_weighting(rate):
    """The two BS.1770 K-weighting biquads (shelf, high-pass) for any rate."""
    import numpy as np

    def biquad(b0, b1, b2, a0, a1, a2):
        return np.array([[b0 / a0, b1 / a0, b2 / a0, 1.0, a1 / a0, a2 / a0]])

    gain, q, freq = 3.999843853973347, 0.7071752369554196, 1681.9744509555319
    amp = 10 ** (gain / 40)
    w0 = 2 * math.pi * freq / rate
    cos, alpha = math.cos(w0), math.sin(w0) / (2 * q)
    root = 2 * math.sqrt(amp) * alpha
    shelf = biquad(
        amp * ((amp + 1) + (amp - 1) * cos + root),
        -2 * amp * ((amp - 1) + (amp + 1) * cos),
        amp * ((amp + 1) + (amp - 1) * cos - root),
        (amp + 1) - (amp - 1) * cos + root,
        2 * ((amp - 1) - (amp + 1) * cos),
        (amp + 1) - (amp - 1) * cos - root,
    )
    q, freq = 0.5003270373253953, 38.13547087613982
    w0 = 2 * math.pi * freq / rate
    cos, alpha = math.cos(w0), math.sin(w0) / (2 * q)
    high = biquad(
        (1 + cos) / 2, -(1 + cos), (1 + cos) / 2, 1 + alpha, -2 * cos, 1 - alpha
    )
    return np.vstack([shelf, high])


def loudness(samples, rate=SAMPLE_RATE):
    """Integrated loudness of mono `samples` in LUFS (ITU-R BS.1770-4 gating)."""
    import numpy as np
    from scipy.signal import sosfilt

    block, step = int(0.4 * rate), int(0.1 * rate)
    weighted = sosfilt(_k_weighting(rate), np.asarray(samples, dtype=np.float64))
    if weighted.size < block:
        weighted = np.concatenate([weighted, np.zeros(block - weighted.size)])
    total = np.concatenate([[0.0], np.cumsum(weighted**2)])
    starts = np.arange(0, weighted.size - block + 1, step)
    energy = (total[starts + block] - total[starts]) / block
    with np.errstate(divide="ignore"):
        levels = -0.691 + 10 * np.log10(energy)
    kept = energy[levels > -70.0]
    if not kept.size:
        return -math.inf
    relative = -0.691 + 10 * math.log10(float(kept.mean())) - 10.0
    kept = energy[levels > max(-70.0, relative)]
    if not kept.size:
        return -math.inf
    return -0.691 + 10 * math.log10(float(kept.mean()))


def master(track, room=ROOM_MIX):
    """Finish the dry model output like a voice-over: remove rumble, level
    loud and soft sentences, add a touch of room and normalize loudness."""
    import numpy as np
    from scipy.signal import butter, fftconvolve, sosfilt

    if not track.size or not np.any(track):
        return track.astype(np.float32)
    voice = sosfilt(
        butter(2, 70, "highpass", fs=SAMPLE_RATE, output="sos"),
        track.astype(np.float64),
    )
    window = int(0.05 * SAMPLE_RATE)
    smooth = np.ones(window) / window
    envelope = np.sqrt(fftconvolve(voice**2, smooth, mode="same") + 1e-12)
    threshold = 10 ** (-24 / 20) * float(np.max(envelope))
    gain = np.ones_like(envelope)
    loud = envelope > threshold
    gain[loud] = (envelope[loud] / threshold) ** (1 / 2.0 - 1)  # 2:1 compression
    voice *= fftconvolve(gain, smooth, mode="same")
    wet = fftconvolve(voice, _room())
    voice = np.concatenate([voice, np.zeros(wet.size - voice.size)]) + room * wet
    measured = loudness(voice)
    if math.isfinite(measured):
        voice *= 10 ** ((TARGET_LUFS - measured) / 20)
    return _limit(voice).astype(np.float32)


def _limit(voice):
    """Soft-limit to CEILING, judged on 4x oversampled audio so the peaks a
    player reconstructs between samples stay below it too."""
    import numpy as np
    from scipy.signal import resample_poly

    knee = 0.8 * CEILING
    for _ in range(4):
        wide = resample_poly(voice, 4, 1)
        if float(np.max(np.abs(wide))) <= CEILING:
            break
        level = np.abs(wide)
        over = level > knee
        wide[over] = np.sign(wide[over]) * (
            knee + (CEILING - knee) * np.tanh((level[over] - knee) / (CEILING - knee))
        )
        voice = resample_poly(wide, 1, 4)[: voice.size]
    peak = float(np.max(np.abs(resample_poly(voice, 4, 1))))
    return voice * (CEILING / peak) if peak > CEILING else voice


def split_text(text: str, limit: int = CHUNK_CHARS) -> list[str]:
    """Cut a paragraph into chunks of at most `limit` characters.

    Prefers sentence ends, then clause marks, then spaces, and only slices a
    word when there is no other choice, so each chunk reads naturally.
    """
    text = text.strip()
    if len(text) <= limit:
        return [text] if text else []
    for pattern in (
        r"(?<=[.!?…])[\"')\]]*\s+",
        r"(?<=[,;:—–])\s+",
        r"\s+",
    ):
        pieces = [piece for piece in re.split(pattern, text) if piece.strip()]
        if len(pieces) < 2:
            continue
        chunks, current = [], ""
        for piece in pieces:
            if len(piece) > limit:
                if current:
                    chunks.append(current)
                    current = ""
                chunks.extend(split_text(piece, limit))
            elif not current:
                current = piece
            elif len(current) + 1 + len(piece) <= limit:
                current = f"{current} {piece}"
            else:
                chunks.append(current)
                current = piece
        if current:
            chunks.append(current)
        return chunks
    return [text[start : start + limit] for start in range(0, len(text), limit)]


def _speak(engine, text, *, voice, speed, lang, warnings, line):
    """Speak one paragraph as mono 24 kHz samples; never fails on unspeakable text."""
    import numpy as np

    pieces = []
    gap = np.zeros(int(SENTENCE_PAUSE * SAMPLE_RATE), dtype=np.float32)
    for chunk in split_text(text):
        try:
            samples, rate = engine.create(
                chunk,
                voice=voice,
                speed=float(speed),
                lang=lang,
                sentence_pause=SENTENCE_PAUSE,
                clause_pause=CLAUSE_PAUSE,
            )
        except ValueError as error:
            # Symbols, emoji or punctuation alone have no phonemes to speak.
            if "phoneme" in str(error).lower() or "synthesize" in str(error).lower():
                warnings.append(
                    f"Narration line {line} has text Kokoro cannot pronounce "
                    f"({chunk[:40]!r}); it was skipped."
                )
                continue
            raise
        if rate != SAMPLE_RATE:
            raise NarrationError("The voice model returned an unexpected sample rate.")
        if pieces:
            pieces.append(gap)
        pieces.append(np.asarray(samples, dtype=np.float32).reshape(-1))
    return np.concatenate(pieces) if pieces else np.zeros(0, dtype=np.float32)


def _line_path(cache_dir, quality, voice, speed, text):
    key = hashlib.sha256(
        json.dumps([CACHE_VERSION, quality, voice, float(speed), text]).encode()
    ).hexdigest()
    return Path(cache_dir) / f"{key}.npy"


def _prune(directory: Path, limit: int):
    """Drop the least recently used files once the cache exceeds `limit` bytes."""
    try:
        files = sorted(
            directory.glob("*.npy"),
            key=lambda path: path.stat().st_mtime,
            reverse=True,
        )
        total = 0
        for path in files:
            total += path.stat().st_size
            if total > limit:
                path.unlink(missing_ok=True)
    except OSError:
        pass


def synthesize(
    segments,
    *,
    voice,
    speed,
    engine=None,
    report=print,
    load=None,
    cache_dir=None,
    quality="standard",
):
    """Return a mastered mono 24 kHz track plus timing warnings.

    Finished lines are checkpointed in `cache_dir`, so after a failure only the
    lines that changed or never finished are generated again. `engine` is
    loaded lazily through `load` so a fully cached narration needs no model.
    """
    import numpy as np

    lang = language(voice)
    placed, warnings, cursor = [], [], 0.0
    for index, segment in enumerate(segments, 1):
        report(f"Generating narration · line {index} of {len(segments)}")
        start = segment["start"]
        if start is None:
            start = cursor + (GAP if index > 1 else 0.0)
        elif start < cursor - 0.05:
            warnings.append(
                f"Narration line {segment['line']} is set to [{format_time(start)}] but the "
                f"previous line is still speaking; it starts at {format_time(cursor)} instead."
            )
            start = cursor
        text = prepare_text(segment["text"]) or segment["text"]
        samples = None
        cached = (
            _line_path(cache_dir, quality, voice, speed, text) if cache_dir else None
        )
        if cached is not None and cached.is_file():
            try:
                samples = np.load(cached, allow_pickle=False)
                cached.touch()
            except (OSError, ValueError):
                samples = None
        if samples is None:
            if engine is None:
                engine = load()
            samples = _shape(
                _speak(
                    engine,
                    text,
                    voice=voice,
                    speed=speed,
                    lang=lang,
                    warnings=warnings,
                    line=segment["line"],
                )
            )
            if cached is not None:
                try:
                    cached.parent.mkdir(parents=True, exist_ok=True)
                    temporary = cached.with_name(uuid.uuid4().hex + ".tmp")
                    with temporary.open("wb") as output:
                        np.save(output, samples, allow_pickle=False)
                    temporary.replace(cached)
                except OSError:
                    pass
        placed.append((int(round(start * SAMPLE_RATE)), samples))
        cursor = start + len(samples) / SAMPLE_RATE
        if cursor > MAX_SECONDS:
            raise NarrationError(
                f"Narration is longer than {int(MAX_SECONDS // 60)} minutes at line "
                f"{segment['line']}. Split it into shorter videos."
            )
    if cache_dir:
        _prune(Path(cache_dir), LINE_CACHE_LIMIT)
    length = max((offset + len(samples) for offset, samples in placed), default=0)
    track = np.zeros(length, dtype=np.float32)
    for offset, samples in placed:
        track[offset : offset + len(samples)] += samples
    return master(track), warnings


def write_wav(path: Path, track):
    import numpy as np
    import wave

    pcm = (np.clip(track, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(SAMPLE_RATE)
        output.writeframes(pcm.tobytes())


def _resampled(frames):
    """Decode or convert frames into a (2, samples) float32 array at OUTPUT_RATE."""
    import av
    import numpy as np

    resampler = av.AudioResampler(format="fltp", layout="stereo", rate=OUTPUT_RATE)
    chunks = []
    for frame in [*frames, None]:
        for converted in resampler.resample(frame):
            chunks.append(converted.to_ndarray())
    return (
        np.concatenate(chunks, axis=1).astype(np.float32)
        if chunks
        else np.zeros((2, 0), dtype=np.float32)
    )


def mux(video: Path, track, output: Path) -> dict:
    """Copy the video stream untouched and add narration mixed with any audio."""
    import av
    import numpy as np

    mono = av.AudioFrame.from_ndarray(
        np.ascontiguousarray(track.reshape(1, -1)), format="flt", layout="mono"
    )
    mono.sample_rate = SAMPLE_RATE
    narration = _resampled([mono]) if track.size else np.zeros((2, 0))
    with av.open(str(video)) as source:
        stream = source.streams.video[0]
        if stream.duration is not None:
            duration = float(stream.duration * stream.time_base)
        else:
            duration = (source.duration or 0) / av.time_base
        existing = np.zeros((2, 0), dtype=np.float32)
        if source.streams.audio:
            existing = _resampled(list(source.decode(source.streams.audio[0])))
    length = max(narration.shape[1], existing.shape[1])
    mixed = np.zeros((2, length), dtype=np.float32)
    mixed[:, : existing.shape[1]] += existing
    mixed[:, : narration.shape[1]] += narration
    peak = float(np.max(np.abs(mixed))) if mixed.size else 0.0
    if peak > 0.98:
        mixed *= 0.98 / peak

    with av.open(str(video)) as source, av.open(str(output), "w", format="mp4") as out:
        in_video = source.streams.video[0]
        out_video = out.add_stream_from_template(in_video)
        out_audio = out.add_stream("aac", rate=OUTPUT_RATE, layout="stereo")
        out_audio.bit_rate = 160_000
        packets = []
        frame_size = 1024
        for offset in range(0, mixed.shape[1], frame_size):
            frame = av.AudioFrame.from_ndarray(
                np.ascontiguousarray(mixed[:, offset : offset + frame_size]),
                format="fltp",
                layout="stereo",
            )
            frame.sample_rate = OUTPUT_RATE
            frame.pts = offset
            packets.extend(out_audio.encode(frame))
        packets.extend(out_audio.encode(None))
        # Interleave by time so the muxer never buffers the whole video.
        pending = iter(packets)
        upcoming = next(pending, None)
        for packet in source.demux(in_video):
            if packet.dts is None:
                continue
            moment = float(packet.dts * packet.time_base)
            while upcoming is not None and (
                upcoming.pts is None
                or float(upcoming.pts * upcoming.time_base) <= moment
            ):
                out.mux(upcoming)
                upcoming = next(pending, None)
            packet.stream = out_video
            out.mux(packet)
        while upcoming is not None:
            out.mux(upcoming)
            upcoming = next(pending, None)
    return {"video": duration, "narration": track.size / SAMPLE_RATE}


def prepare_track(spec: dict, report=print):
    """Synthesize the narration before the animation renders.

    Problems with the script or voice then surface in seconds instead of after
    a long render, and finished lines are checkpointed for the next attempt.
    Returns (track, warnings), or None when there is nothing to say.
    """
    segments = parse_timeline(spec["text"])
    if not segments:
        return None
    data_root = Path(spec["data_root"])

    def load():
        report("Loading the narration voice…")
        return load_engine(data_root, spec["quality"])

    return synthesize(
        segments,
        voice=spec["voice"],
        speed=spec["speed"],
        load=load,
        report=report,
        cache_dir=spec.get("line_cache"),
        quality=spec["quality"],
    )


def restore_video(spec: dict, report=print) -> bool:
    """Reuse the animation checkpointed by an earlier identical render."""
    cache = spec.get("video_cache")
    if not cache:
        return False
    cached = Path(cache) / "video.mp4"
    try:
        if not cached.is_file() or cached.stat().st_size == 0:
            return False
        target = Path(spec["media_dir"]) / "videos" / "checkpoint" / "video.mp4"
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(cached, target)
        os.utime(cached)
    except OSError:
        return False
    report("Reusing the animation from your previous render…")
    return True


def checkpoint_video(spec: dict):
    """Save the silent animation so a narration failure never repeats Manim."""
    cache = spec.get("video_cache")
    if not cache:
        return
    videos = _rendered_videos(spec)
    if not videos:
        return
    video = max(videos, key=lambda path: path.stat().st_mtime_ns)
    directory = Path(cache)
    try:
        if video.stat().st_size > VIDEO_CACHE_MAX_BYTES:
            return
        directory.mkdir(parents=True, exist_ok=True)
        temporary = directory / (uuid.uuid4().hex + ".tmp")
        shutil.copy2(video, temporary)
        temporary.replace(directory / "video.mp4")
        siblings = sorted(
            (path for path in directory.parent.iterdir() if path.is_dir()),
            key=lambda path: path.stat().st_mtime,
            reverse=True,
        )
        for old in siblings[VIDEO_CACHE_KEEP:]:
            shutil.rmtree(old, ignore_errors=True)
    except OSError:
        pass


def _rendered_videos(spec: dict) -> list[Path]:
    return [
        path
        for path in Path(spec["media_dir"]).rglob("video.mp4")
        if "partial_movie_files" not in path.parts
    ]


def narrate_render(spec: dict, report=print, warn=print, prepared=None):
    """Called by the render worker once the (silent) video exists."""
    if prepared is None:
        prepared = prepare_track(spec, report)
    if prepared is None:
        return
    track, warnings = prepared
    videos = _rendered_videos(spec)
    if not videos:
        return
    video = max(videos, key=lambda path: path.stat().st_mtime_ns)
    report("Adding narration to your video…")
    staged = video.with_name("narrated.mp4")
    try:
        durations = mux(video, track, staged)
        staged.replace(video)
    finally:
        staged.unlink(missing_ok=True)
    overrun = durations["narration"] - durations["video"]
    if overrun > 0.1:
        warnings.append(
            f"Narration runs {overrun:.1f}s past the animation, so the last frame holds "
            f"while it finishes. Add self.wait({math.ceil(overrun)}) at the end of your scene "
            "to show the final frame on purpose."
        )
    for message in warnings:
        warn(message)


def main():
    """Short-lived preview process so the server never keeps the model loaded."""
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["preview"])
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    spec = json.loads(args.spec.read_text(encoding="utf-8"))
    segments = parse_timeline(spec["text"])[:1] or [
        {"start": None, "text": "This is how your narration will sound.", "line": 1}
    ]
    for segment in segments:
        segment["start"] = None
    engine = load_engine(Path(spec["data_root"]), spec["quality"])
    track, _ = synthesize(
        segments,
        voice=spec["voice"],
        speed=spec["speed"],
        engine=engine,
        report=lambda message: None,
        cache_dir=spec.get("line_cache"),
        quality=spec["quality"],
    )
    write_wav(args.output, track)


if __name__ == "__main__":
    try:
        main()
    except NarrationError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)
