// Style showcase: the title card from the house-style board, built in about four
// seconds. The studio renders it once per house style for the style pickers (a still
// at the "thumbnail" mark plus a short looping clip). Everything comes from the theme.
import {
  Scene, Text, MathTex, Sector, Line, Dot,
  FadeIn, Write, Create, GrowFromCenter, LaggedStart,
  LEFT, RIGHT, UP, DOWN, ORIGIN, TAU, vec,
} from "manim";

export default class Showcase extends Scene {
  construct() {
    const t = this.theme ?? {};
    const c = {
      background: "#FFFFFF", text: "#1E2731", muted: "#68717D", primary: "#3B5BDB",
      secondary: "#0C8599", accent: "#E8590C",
      ...(t.colors ?? {}),
    };
    const m = t.layout?.margin ?? 0.65;
    const centered = t.layout?.title === "top-center" || t.layout?.title === "center";
    const W = this.frameWidth;
    const titleSize = (t.sizes?.title ?? 54) * 1.3;
    const textWidth = centered ? W - 2 * m - 1 : W * 0.5;
    const edge = centered ? ORIGIN : LEFT;

    const title = Text("The area of a circle", { role: "title", fontSize: titleSize, maxWidth: textWidth, align: centered ? "center" : "left" });
    const eyebrow = Text("CHAPTER 3 · GEOMETRY", { role: "caption", color: c.primary, weight: "semibold" });
    const sub = Text("Why the answer is πr², shown by cutting a circle into wedges.", {
      role: "body", color: c.muted, maxWidth: centered ? W * 0.62 : W * 0.44, align: centered ? "center" : "left",
    });
    const note = Text("spoiler: you can unroll it", { role: "hand" });

    // Stack the lines by hand (each piece animates on its own, so no group is involved).
    eyebrow.nextTo(title, UP, 0.38, edge);
    sub.nextTo(title, DOWN, 0.38, edge);
    note.nextTo(sub, DOWN, 0.5, edge);
    const parts = [eyebrow, title, sub, note];
    const top = eyebrow.getTop().y, bottom = note.getBottom().y;
    const x = centered ? -title.getCenter().x : -W / 2 + m + 0.35 - title.getLeft().x;
    for (const p of parts) p.shift(vec(x, -(top + bottom) / 2));
    if (!centered) note.rotate(0.05);

    this.play(FadeIn(eyebrow, { shift: UP.mul(0.15) }), { runTime: 0.45 });
    this.play(Write(title), { runTime: 1.0 });

    if (centered) {
      this.play(FadeIn(sub, { shift: UP.mul(0.15) }), { runTime: 0.7 });
    } else {
      const R = 2.0;
      const center = vec(W / 2 - m - R - 0.45, 0);
      const n = 12;
      const wedges = Array.from({ length: n }, (_, i) =>
        Sector({
          radius: R,
          startAngle: TAU / 4 - ((i + 1) * TAU) / n,
          angle: TAU / n,
          fillColor: i % 2 ? c.secondary : c.primary,
          fillOpacity: i % 2 ? 0.55 : 0.85,
          strokeColor: c.background,
          strokeWidth: 3,
        }).shift(center),
      );
      const radius = Line(center, center.add(RIGHT.mul(R)), { color: c.text, strokeWidth: 4 });
      const dot = Dot(center, { color: c.text, radius: 0.06 });
      const r = MathTex("r", { fontSize: 36, color: c.text }).nextTo(radius, UP, 0.08);
      this.play(
        FadeIn(sub, { shift: UP.mul(0.15) }),
        LaggedStart(...wedges.map((w) => GrowFromCenter(w)), { lagRatio: 0.08 }),
        { runTime: 1.2 },
      );
      this.play(Create(radius), FadeIn(dot), FadeIn(r), { runTime: 0.4 });
    }
    this.play(Write(note), { runTime: 0.7 });
    this.mark("thumbnail");
    this.wait(0.8);
  }
}
