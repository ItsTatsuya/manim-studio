"""Run with bundled Python and a fresh MANIM_STUDIO_DATA to verify offline math."""

import shutil
import time
import studio

source = """from manim import *
class MainScene(Scene):
    def construct(self):
        self.add(Text("Bundled math", font_size=30).to_edge(UP))
        self.play(Write(MathTex(r"e^{i\\pi}+1=0")), run_time=0.5)
        self.wait(0.2)
"""
print("Bundled LaTeX:", shutil.which("latex"), flush=True)
assert shutil.which("latex")
result = studio.start_render(
    studio.RenderRequest(
        source=source, scene="MainScene", name="Offline equation check"
    )
)
deadline = time.monotonic() + 120
while time.monotonic() < deadline:
    job = studio.JOBS[result["id"]]
    if job["status"] in {"completed", "failed", "cancelled"}:
        break
    time.sleep(0.1)
else:
    studio.cancel_active()
    raise AssertionError("Equation render timed out")
assert job["status"] == "completed", job
assert job["size"] > 0
print("PASS: standalone Text and MathTex render", flush=True)
