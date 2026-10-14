"""Kokoro-82M narration: verified model files, timeline, synthesis and muxing.

Kokoro runs on the CPU through ONNX Runtime, so no GPU or PyTorch is needed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import sys
import threading
import urllib.request

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
TARGET_RMS_DB = -20.0
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


def master(track):
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
    envelope = np.sqrt(np.convolve(voice**2, smooth, mode="same") + 1e-12)
    threshold = 10 ** (-24 / 20) * float(np.max(envelope))
    gain = np.ones_like(envelope)
    loud = envelope > threshold
    gain[loud] = (envelope[loud] / threshold) ** (1 / 2.0 - 1)  # 2:1 compression
    voice *= np.convolve(gain, smooth, mode="same")
    wet = fftconvolve(voice, _room())
    voice = np.concatenate([voice, np.zeros(wet.size - voice.size)]) + 0.07 * wet
    frames = voice[: voice.size // window * window].reshape(-1, window)
    energy = np.sqrt(np.mean(frames**2, axis=1)) if frames.size else np.zeros(1)
    speech = energy[energy > float(energy.max()) * 10 ** (-40 / 20)]
    rms = float(np.sqrt(np.mean(speech**2))) if speech.size else 0.0
    if rms > 0:
        voice *= 10 ** (TARGET_RMS_DB / 20) / rms
    # Soft limiter: transparent below the knee, smoothly approaching the ceiling.
    knee, ceiling = 0.7, 0.95
    level = np.abs(voice)
    over = level > knee
    voice[over] = np.sign(voice[over]) * (
        knee + (ceiling - knee) * np.tanh((level[over] - knee) / (ceiling - knee))
    )
    return voice.astype(np.float32)


def synthesize(segments, *, voice, speed, engine, report=print):
    """Return a mastered mono 24 kHz track plus timing warnings."""
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
        samples, rate = engine.create(
            text,
            voice=voice,
            speed=float(speed),
            lang=lang,
            sentence_pause=SENTENCE_PAUSE,
            clause_pause=CLAUSE_PAUSE,
            # Overlapping windows keep intonation flowing across long paragraphs.
            continuous=len(text) > 300,
        )
        if rate != SAMPLE_RATE:
            raise NarrationError("The voice model returned an unexpected sample rate.")
        samples = _shape(np.asarray(samples, dtype=np.float32).reshape(-1))
        placed.append((int(round(start * SAMPLE_RATE)), samples))
        cursor = start + len(samples) / SAMPLE_RATE
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


def narrate_render(spec: dict, report=print, warn=print):
    """Called by the render worker after Manim writes its video."""
    segments = parse_timeline(spec["text"])
    if not segments:
        return
    media = Path(spec["media_dir"])
    videos = [
        path
        for path in media.rglob("video.mp4")
        if "partial_movie_files" not in path.parts
    ]
    if not videos:
        return
    video = max(videos, key=lambda path: path.stat().st_mtime_ns)
    report("Loading the narration voice…")
    engine = load_engine(Path(spec["data_root"]), spec["quality"])
    track, warnings = synthesize(
        segments, voice=spec["voice"], speed=spec["speed"], engine=engine, report=report
    )
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
    )
    write_wav(args.output, track)


if __name__ == "__main__":
    try:
        main()
    except NarrationError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2)
