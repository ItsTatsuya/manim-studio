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
