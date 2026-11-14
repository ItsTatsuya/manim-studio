// Scene 3 · Unroll the rings
// Each thin ring of radius r straightens into a strip of length 2πr. Stacked from the
// outside in, the strips make a triangle with base 2πR and height R.
import {
  Scene, Text, MathTex, Circle, Line, DashedLine, Brace,
  Create, Transform, Write, FadeIn, GrowFromCenter, DOWN, LEFT, PI, vec,
} from "manim";
import { palette } from "../lib/kit";

export default class Unroll extends Scene {
  construct() {
    const c = palette(this.theme);
    const colors = [c.primary, c.secondary, c.accent, c.positive];
    const R = 1.8;
    const n = 6;
    const thick = R / n;
    const center = vec(0, 1.55);
    const baseY = -2.4;

    const rings = Array.from({ length: n }, (_, i) => {
      const r = (i + 0.5) * thick;
      return Circle(r, { color: colors[i % colors.length], strokeWidth: thick * 92 }).moveTo(center);
    });
    this.narrate(1);
    this.play(...rings.map((ring) => Create(ring)), { runTime: 1.5 });
    this.narrate(2);

    // Outermost ring first: it becomes the base of the triangle.
    this.narrate(3);
    for (let i = n - 1; i >= 0; i--) {
      if (i === Math.floor(n / 2)) this.narrate(4);
      const r = (i + 0.5) * thick;
      const y = baseY + (R - r);
      const strip = Line([-PI * r, y], [PI * r, y], { color: colors[i % colors.length], strokeWidth: thick * 92 });
      this.play(Transform(rings[i], strip), { runTime: 1.6 });
      this.wait(0.3);
    }
    this.narrate(5);

    const base = Brace(rings[n - 1], DOWN, { color: c.text });
    const baseLabel = MathTex("2 pi r", { fontSize: 40, color: c.text }).nextTo(base, DOWN, 0.15);
    this.narrate(6);
    this.play(GrowFromCenter(base), Write(baseLabel));
    this.wait(1);

    // The height, measured beside the stack so it doesn't sit on the strips.
    const x = -PI * R - 0.3;
    const height = DashedLine([x, baseY], [x, baseY + R], { color: c.text, strokeWidth: 3 });
    const heightLabel = MathTex("r", { fontSize: 40, color: c.text }).nextTo(height, LEFT, 0.15);
    this.narrate(7);
    this.play(Create(height), Write(heightLabel));
    this.wait(1);

    const idea = Text("The strips stack into a triangle", { role: "heading" }).moveTo(vec(0, 2.6));
    const sub = Text("base = circumference, height = radius", { role: "caption" }).nextTo(idea, DOWN, 0.25);
    this.play(FadeIn(idea, { shift: DOWN.mul(0.2) }));
    this.play(FadeIn(sub));
    this.mark("thumbnail");
    this.wait(1.5);
  }
}
