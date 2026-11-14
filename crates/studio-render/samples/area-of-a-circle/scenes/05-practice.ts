// Scene 5 · Practice question
import {
  Scene, Text, MathTex, Integer, ValueTracker, alwaysRedraw,
  Write, FadeIn, FadeOut, LaggedStart, Indicate, UP, DOWN, LEFT, RIGHT, vec,
} from "manim";
import { palette, heading, card } from "../lib/kit";

export default class Practice extends Scene {
  construct() {
    const t = new ValueTracker(5);
    const c = palette(this.theme);
    const title = heading("Your turn");
    this.narrate(1);
    this.play(Write(title));

    const question = Text("A circle has a radius of 3 cm. What is its area?", { role: "body" }).moveTo(vec(0, 1.6));
    this.narrate(2);
    this.play(FadeIn(question, { shift: UP.mul(0.15) }));

    const a = card(Text("A ≈ 28.3 cm²", { role: "body" }), c.key_fill, c.muted);
    const b = card(Text("A ≈ 18.8 cm²", { role: "body" }), c.key_fill, c.muted);
    a.moveTo(vec(-2.4, -0.2));
    b.moveTo(vec(2.4, -0.2));
    this.narrate(3);
    this.play(LaggedStart(FadeIn(a, { shift: UP.mul(0.2) }), FadeIn(b, { shift: UP.mul(0.2) }), { lagRatio: 0.3 }));

    const pause = Text("Pause the video and work it out", { role: "caption" }).moveTo(vec(0, -2.1));
    this.narrate(4);
    this.play(FadeIn(pause));

    // A 5-second countdown.
    const count = alwaysRedraw(() =>
      Integer(Math.max(1, Math.ceil(t.getValue() - 1e-6)), { role: "heading", color: c.muted }).nextTo(pause, DOWN, 0.3),
    );
    this.add(count);
    this.play(t.animate.setValue(0), { runTime: 5, rateFunc: "linear" });
    this.remove(count);
    this.play(FadeOut(pause), { runTime: 0.4 });

    // The right answer turns green; the other fades back.
    this.narrate(5);
    this.play(
      a.get(0).animate.setFill(c.positive, 0.25).setStroke(c.positive, 5),
      b.animate.setOpacity(0.35),
      { runTime: 0.8 },
    );
    this.play(Indicate(a, { color: c.positive, scaleFactor: 1.05 }));

    const working = MathTex("A = pi dot 3^2 = 9 pi approx 28.3 \"cm\"^2", { fontSize: 48, color: c.text }).moveTo(vec(0, -2.3));
    this.narrate(6);
    this.play(Write(working), { runTime: 2 });
    this.mark("thumbnail");
    this.wait(1);
    this.play(FadeOut(working), FadeOut(b, { shift: RIGHT.mul(0.3) }), FadeOut(a, { shift: LEFT.mul(0.3) }), FadeOut(question));
    const end = Text("Well done!", { role: "title" });
    this.narrate(7);
    this.play(FadeIn(end, { scale: 0.9 }));
    this.wait(2);
  }
}
