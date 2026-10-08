"""Exercise bundled FFmpeg, ffprobe, pydub and Manim's audio export."""

import time
import numpy as np
import av
import studio
from runtime import DATA_ROOT

tone = DATA_ROOT / "assets" / "tone.mp3"
rate = 44100
samples = (np.sin(2 * np.pi * 440 * np.arange(rate // 2) / rate) * 8000).astype(
    np.int16
)
with av.open(str(tone), "w") as output:
    stream = output.add_stream("libmp3lame", rate=rate)
    frame = av.AudioFrame.from_ndarray(
        samples.reshape(1, -1), format="s16", layout="mono"
    )
    frame.sample_rate = rate
    for packet in stream.encode(frame):
        output.mux(packet)
    for packet in stream.encode(None):
        output.mux(packet)
source = """from manim import *
class MainScene(Scene):
    def construct(self):
        self.add_sound("assets/tone.mp3")
        self.add(Circle())
        self.wait(0.7)
"""
job = studio.start_render(
    studio.RenderRequest(source=source, scene="MainScene", name="Bundled audio check")
)
deadline = time.monotonic() + 90
while time.monotonic() < deadline:
    result = studio.JOBS[job["id"]]
    if result["status"] in {"completed", "failed", "cancelled"}:
        break
    time.sleep(0.1)
else:
    studio.cancel_active()
    raise AssertionError("Audio render timed out")
assert result["status"] == "completed", result
with av.open(str(studio.output_directory(job) / "video.mp4")) as video:
    assert video.streams.audio
    assert next(video.decode(audio=0)).samples > 0
print("PASS: bundled MP3 decode and video with audio", flush=True)
