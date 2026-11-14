// Scene 4 · The formula
import { Scene, Text, MathTex, SurroundingRectangle, Write, FadeIn, FadeOut, ReplacementTransform, Create, Indicate, DOWN, UP } from "manim";
import { palette, heading } from "../lib/kit";

export default class Formula extends Scene {
  construct() {
    const c = palette(this.theme);
    const title = heading("The formula");
    this.narrate(1);
    this.play(Write(title));

    const size = 60;
    const step1 = MathTex(`"Area" = 1/2 dot "base" dot "height"`, { fontSize: size, color: c.text });
    this.narrate(2);
    this.play(Write(step1), { runTime: 2 });

    const step2 = MathTex("A = 1/2 dot 2 pi r dot r", { fontSize: size, color: c.text });
    this.narrate(3);
    this.play(ReplacementTransform(step1, step2), { runTime: 1.5 });

    const step3 = MathTex("A = pi r dot r", { fontSize: size, color: c.text });
    this.narrate(4);
    this.play(ReplacementTransform(step2, step3), { runTime: 1.5 });

    const result = MathTex("A = pi r^2", { fontSize: 84, color: c.primary });
    this.narrate(5);
    this.play(ReplacementTransform(step3, result), { runTime: 1.5 });
    const box = SurroundingRectangle(result, { buff: 0.35, cornerRadius: 0.15, color: c.positive, strokeWidth: 6 });
    this.play(Create(box));
    this.play(Indicate(result, { color: c.accent, scaleFactor: 1.08 }));

    const note = Text("π ≈ 3.14, so the area is a little more than 3 squares of side r", { role: "caption", maxWidth: 10 })
      .nextTo(box, DOWN, 0.6);
    this.narrate(6);
    this.play(FadeIn(note, { shift: UP.mul(0.15) }));
    this.mark("thumbnail");
    this.wait(1.5);
    this.play(FadeOut(note), { runTime: 0.5 });
    this.wait(0.5);
  }
}
