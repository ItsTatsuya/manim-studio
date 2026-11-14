// Scene 1 · Hook: pizza slices
import { Scene, Text, Circle, Sector, Write, FadeIn, FadeOut, DrawBorderThenFill, LaggedStart, UP, DOWN, TAU, vec } from "manim";
import { palette } from "../lib/kit";

export default class Hook extends Scene {
  construct() {
    const c = palette(this.theme);

    const name = Text("Area of a circle", { role: "title", fontSize: 72 });
    const sub = Text("Grade 8 · Geometry", { role: "caption" }).nextTo(name, DOWN, 0.4);
    this.narrate(1);
    this.play(Write(name), FadeIn(sub, { shift: UP.mul(0.2) }));
    this.wait(1.5);
    this.play(FadeOut(name, { shift: UP.mul(0.3) }), FadeOut(sub, { shift: UP.mul(0.3) }), { runTime: 0.6 });

    const R = 2.1;
    const pizza = Circle(R, { color: c.primary, fillColor: c.primary, fillOpacity: 0.2, strokeWidth: 6 }).shift(UP.mul(0.4));
    this.narrate(2);
    this.play(DrawBorderThenFill(pizza), { runTime: 1.4 });

    const slices = Array.from({ length: 8 }, (_, i) =>
      Sector({
        radius: R,
        startAngle: (i * TAU) / 8,
        angle: TAU / 8,
        fillColor: i % 2 ? c.accent : c.primary,
        fillOpacity: 0.6,
        strokeColor: c.background,
        strokeWidth: 5,
      }).shift(UP.mul(0.4)),
    );
    this.narrate(3);
    this.play(FadeOut(pizza), LaggedStart(...slices.map((s) => FadeIn(s)), { lagRatio: 0.12 }), { runTime: 1.6 });

    // Pull the slices apart a little, like a cut pizza.
    this.play(
      ...slices.map((s, i) => {
        const mid = ((i + 0.5) * TAU) / 8;
        return s.animate.shift(vec(Math.cos(mid), Math.sin(mid)).mul(0.22));
      }),
      { runTime: 0.8 },
    );

    const question = Text("Double the radius… do you get twice the pizza?", { role: "heading" }).toEdge(DOWN, 0.6);
    this.narrate(4);
    this.play(Write(question), { runTime: 1.6 });
    this.mark("thumbnail");
    this.narrate(5);
    this.wait(1);
  }
}
