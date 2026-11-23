# Engine issues to report (manim-rust)

Found while building Manim Studio. Send these to the engine session when it is running.

## Animating a Text that belongs to a VGroup draws nothing

- Engine pin: `122b41465bfc8901b2eec9dc77e69177374d5440`
- Found: 2026-11-14, building the style showcase (`crates/studio-render/samples/style-showcase/showcase.ts`)

Text that is placed in a `VGroup` (for example to use `arrange`) never appears when it is
animated on its own. The same text outside a group renders. Shapes are unaffected.

```ts
import { Scene, Text, VGroup, FadeIn, DOWN, ORIGIN } from "manim";
export default class T extends Scene {
  construct() {
    const a = Text("Plain text no group", { role: "title" }).shift(DOWN);   // visible
    const b = Text("Grouped", { role: "title" });                           // never visible
    const c = Text("Grouped two", { role: "body" });
    VGroup(b, c).arrange(DOWN).moveTo(ORIGIN).shift([0, 2, 0]);
    this.play(FadeIn(a), FadeIn(b));
    this.wait(0.5);
  }
}
```

Expected: both "Plain text no group" and "Grouped" are drawn. Actual: only the first.
`b.getLeft()` reports a sensible position, so layout works and only drawing is missing.

Workaround in the studio: position the pieces with `nextTo` and shift them individually.

**Status (2026-11-14):** fixed in manim-rust `752eb20`, pushed and pinned in Cargo.toml (repro verified).
Cause: playing a member of an off-screen group lent it to the scene for the play only.
The showcase keeps its manual layout, which avoids the caveat below.
Note: while a member is on screen on its own, the group's layout calls don't see it
until the group itself joins the scene.

## Polish brief, workstream 4: camera, card effects, dash phase

- Engine: manim-rust PR #1 (`polish/4-engine`), pinned here at `89286911`.
- Found: 2026-11-23, from the S03-S04 reference scene (`Scene.tsx`).

What the reference does that the engine could not:

1. **Camera.** The reference zooms only the graph: scale about 1.22 about the crossing (10, 20),
   which glides to the middle of the figure panel, eased over 2.2 s and back over 1.8 s, while the
   work column and the watermark stay put and the card clips the zoomed plot. A plain
   `MovingCameraScene` frame (which the brief lists) moves everything, so the engine also needed
   mobjects that ignore the camera (`pin()`), a clip rectangle (`clipTo()`), and a
   "zoom on a point while it glides to an anchor" move (`MoveCamera({ focus })`). Done.
2. **Card effects.** Soft drop shadows (`setShadow`), linear gradient fills (`setGradient`) and a
   background blur (`setBackdropBlur`) for the full-width card's stage. Rounded rectangles already
   existed. These frames are drawn by tiny-skia on every renderer, so CPU and GPU match by
   construction; the cost is that the GPU renderer does not speed those frames up. Done.
3. **Dash phase.** `DashedLine` baked its dashes into the geometry, so nothing could animate. Dashes
   are now a style cut at draw time (`setDash(length, gap, phase)`): `Create` draws a dashed line on,
   `animate.setDash(..., phase)` makes it march, and it works on curves. Done.

Also found and fixed in the same PR:

- Script `Axes.c2p` / `p2c` ignored scaling of the axes after creation (only moves were followed).
- Script `group.get(i)` returned a new wrapper and handle on every call (`g.get(0) !== g.get(0)`);
  the lesson kit worked around it by matching mobjects by kind, place and size. The workaround stays
  (it is harmless), but `same()` in `lesson.ts` no longer has to carry it.

Not done: a fuzz case for the new styles, and GPU rendering of shadows and blur (they run on the CPU).
