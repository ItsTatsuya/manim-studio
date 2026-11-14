// Scene 2 · Area grows faster than radius
import {
  Scene, Text, MathTex, Axes, ParametricFunction, Dot, DashedLine, ValueTracker, DecimalNumber, alwaysRedraw,
  Create, Write, FadeIn, UP, DOWN, LEFT, RIGHT, PI, vec,
} from "manim";
import { palette, heading } from "../lib/kit";

export default class Growth extends Scene {
  construct() {
    const r = new ValueTracker(1);
    const c = palette(this.theme);
    const title = heading("Area grows faster than radius");
    this.play(Write(title));

    const axes = Axes({
      xRange: [0, 4, 1],
      yRange: [0, 50, 10],
      xLength: 6.4,
      yLength: 4.4,
      axisColor: c.muted,
      numbers: true,
      center: [-2.2, -0.75],
    });
    const xLabel = MathTex("r", { color: c.muted, fontSize: 36 }).nextTo(axes.c2p(4, 0), RIGHT, 0.2);
    const yLabel = MathTex("A", { color: c.muted, fontSize: 36 }).nextTo(axes.c2p(0, 50), UP, 0.2);
    this.narrate(1);
    this.play(Create(axes), FadeIn(xLabel), FadeIn(yLabel), { runTime: 1.5 });

    const curve = ParametricFunction((t) => axes.c2p(t, PI * t * t), { tRange: [0, 3.95], color: c.primary, strokeWidth: 6 });
    this.play(Create(curve), { runTime: 2 });

    const dot = alwaysRedraw(() => Dot(axes.c2p(r.getValue(), PI * r.getValue() ** 2), { color: c.accent, radius: 0.1 }));
    const drop = alwaysRedraw(() =>
      DashedLine(axes.c2p(r.getValue(), 0), axes.c2p(r.getValue(), PI * r.getValue() ** 2), { color: c.accent, strokeWidth: 3 }),
    );
    const panelX = 3.9;
    const rText = alwaysRedraw(() =>
      DecimalNumber(r.getValue(), { numDecimalPlaces: 1, role: "body" }).moveTo(vec(panelX + 0.9, 0.9)),
    );
    const aText = alwaysRedraw(() =>
      DecimalNumber(PI * r.getValue() ** 2, { numDecimalPlaces: 1, role: "body", color: c.accent }).moveTo(vec(panelX + 0.9, -0.1)),
    );
    const rName = Text("radius", { role: "caption" }).moveTo(vec(panelX - 0.9, 0.9));
    const aName = Text("area", { role: "caption" }).moveTo(vec(panelX - 0.9, -0.1));
    this.add(drop, dot, rText, aText);
    this.play(FadeIn(rName), FadeIn(aName), { runTime: 0.6 });
    this.wait(1);

    this.narrate(2);
    this.play(r.animate.setValue(2), { runTime: 2.5 });
    this.narrate(3);
    this.narrate(4);
    this.play(r.animate.setValue(3.5), { runTime: 3 });
    this.wait(1);

    const note = Text("Double r → four times the area", { role: "body", color: c.primary, maxWidth: 5.2 }).moveTo(vec(4.3, -2.0));
    this.narrate(5);
    this.play(r.animate.setValue(2), { runTime: 1.5 });
    this.play(Write(note));
    this.mark("thumbnail");
    this.wait(1.5);
  }
}
