"""Run with the bundle's Python and fresh data; works without a TeX runtime."""

import time
import studio

source = """from manim import *
class MainScene(Scene):
    def construct(self):
        self.add(Typst("Offline Typst", font_size=30).to_edge(UP))
        self.play(Write(MathTypst("e^(i pi) + 1 = 0")), run_time=0.5)
        self.wait(0.2)
"""
assert studio.health()["typst"]
analysis = studio.analyze(source)
assert analysis["needs_typst"] and not analysis["needs_latex"]
result = studio.start_render(
    studio.RenderRequest(source=source, scene="MainScene", name="Offline Typst check")
)
deadline = time.monotonic() + 120
while time.monotonic() < deadline:
    job = studio.JOBS[result["id"]]
    if job["status"] in {"completed", "failed", "cancelled"}:
        break
    time.sleep(0.1)
else:
    studio.cancel_active()
    raise AssertionError("Typst render timed out")
assert job["status"] == "completed", job
assert job["size"] > 0
print("PASS: standalone Typst and MathTypst render", flush=True)
