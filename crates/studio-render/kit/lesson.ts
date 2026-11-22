// Studio lesson kit v1. Studio keeps this file up to date; change scenes, not this file.
//
// The pieces lessons are made of, each with its own entrance and emphasis, placed through
// the layout regions. `const k = lesson(this);` once per scene, then e.g.
// `const eq = k.EquationCard("3 r + 2 c = 70", k.c.primary, "Order 1");`.
import {
  AnimationGroup, Circle, Create, DashedLine, Dot, FadeIn, FadeOut, GrowFromCenter, Line, MarkupText, MathTex, MoveAlongPath,
  Polygon, Rectangle, RoundedRectangle, ShowPassingFlash, Succession, SurroundingRectangle, Text, VGroup, Wait, alwaysRedraw,
  DOWN, LEFT, RIGHT, UP, type Animation, type Mobject, type Scene,
} from "manim";
import { palette, type Palette } from "./kit";
import { figure, full, work, type Region } from "./layout";

type Point2 = [number, number];

/** Distance between two scene points. */
const dist = (a: number[], b: number[]) => Math.hypot(a[0] - b[0], a[1] - b[1]);

/**
 * Whether two wrappers are the same mobject. Children come back as new wrappers (and
 * handles) each time they are asked for, so they are matched by kind, place and size.
 */
const same = (a: Mobject, b: Mobject) =>
  (a as any)._h === (b as any)._h ||
  (a.kind === b.kind && dist(a.getCenter(), b.getCenter()) < 1e-6 && Math.abs(a.width - b.width) < 1e-6 && Math.abs(a.height - b.height) < 1e-6);

/** The text pieces inside a mobject (an axis's numbers). */
function numbers(m: Mobject): Mobject[] {
  const out: Mobject[] = [];
  const walk = (x: Mobject) => {
    if (x.kind === "Text" || x.kind === "Math" || x.kind === "DecimalNumber" || x.kind === "Integer") out.push(x);
    else for (const child of x.submobjects) walk(child);
  };
  walk(m);
  return out;
}
type Fn = (x: number) => number;

/** What a piece adds to a mobject. */
type Piece<T> = Mobject & T;

export function lesson(scene: Scene) {
  const c: Palette = palette(scene.theme);
  const POP = { rateFunc: "ease_out_back" as const };
  /** How far the stage fades behind a statement card. */
  const DIM = 0.15;
  /** Parts kept hidden inside something on screen (axis numbers a crossing replaced). */
  const hidden: Mobject[] = [];

  /** A rounded panel behind `content`, with an optional coloured bar down its left side. */
  const panel = (content: Mobject, opts: { fill?: string; stroke?: string; bar?: string; padX?: number; padY?: number } = {}) => {
    const padX = opts.padX ?? 0.35, padY = opts.padY ?? 0.22;
    const box = RoundedRectangle({
      width: content.width + 2 * padX + (opts.bar ? 0.12 : 0),
      height: content.height + 2 * padY,
      cornerRadius: 0.14,
      fillColor: opts.fill ?? c.key_fill,
      fillOpacity: 1,
      strokeColor: opts.stroke ?? c.key_fill,
      strokeWidth: opts.stroke ? 2.5 : 0,
    }).moveTo(content);
    if (opts.bar) box.shift([-0.06, 0, 0]);
    const parts: Mobject[] = [box];
    if (opts.bar) {
      parts.push(Rectangle({ width: 0.09, height: box.height - 0.12, fillColor: opts.bar, fillOpacity: 1, strokeWidth: 0 }).moveTo(box).alignTo(box, LEFT).shift([0.06, 0, 0]));
    }
    return { box, group: VGroup(...parts, content) };
  };

  /** An equation in a card with a colour bar and a small tag ("ORDER 1"). enter(), focus(), dim(). */
  function EquationCard(tex: string, colour: string, tag = "") {
    const eq = MathTex(tex, { color: c.text });
    const content = tag
      ? VGroup(Text(tag.toUpperCase(), { role: "caption", color: colour, weight: "semibold" }), eq).arrange(DOWN, { buff: 0.12, alignedEdge: LEFT })
      : eq;
    const { box, group } = panel(content, { bar: colour });
    return Object.assign(group, {
      eq,
      enter: (): Animation => FadeIn(group, { shift: [0.3, 0, 0] }),
      /** Bring it forward: full strength and a flash round its edge. */
      focus: (): Animation =>
        AnimationGroup(group.animate.setOpacity(1), ShowPassingFlash(SurroundingRectangle(box, { color: colour, buff: 0.02, cornerRadius: 0.14, strokeWidth: 4 }))),
      /** Step back while something else is talked about. */
      dim: (): Animation => group.animate.setOpacity(0.4),
    }) as Piece<{ eq: Mobject; enter(): Animation; focus(): Animation; dim(): Animation }>;
  }

  /** Numbered worked steps, pushed into `region` (work by default) one at a time: `this.play(steps.next())`. */
  function Steps(texs: string[], region: Region = work) {
    const rows = texs.map((t, i) =>
      VGroup(Text(String(i + 1), { role: "caption", color: c.muted, weight: "semibold" }), MathTex(t, { color: c.text })).arrange(RIGHT, { buff: 0.3 }),
    );
    let shown = 0;
    return {
      rows,
      get left() { return rows.length - shown; },
      /** The next step slides in under the last; nothing (a short wait) once all are shown. */
      next(): Animation {
        const row = rows[shown++];
        if (!row) return Wait(0.1);
        region.push(row);
        return FadeIn(row, { shift: [0.25, 0, 0] });
      },
    };
  }

  /** The final answer in a rounded box, green unless told otherwise, with an optional line under it. */
  function AnswerBox(tex: string, sub = "", colour: string = c.positive) {
    const eq = MathTex(tex, { color: c.text });
    const content = sub ? VGroup(eq, Text(sub, { role: "caption", color: c.muted })).arrange(DOWN, { buff: 0.14 }) : eq;
    const { group } = panel(content, { fill: c.background, stroke: colour, padX: 0.45, padY: 0.28 });
    return Object.assign(group, {
      enter: (): Animation => FadeIn(group, { scale: 0.85, ...POP }),
    }) as Piece<{ enter(): Animation }>;
  }

  /** One idea in a line ("One equation = one line of answers."), with an optional smaller line. */
  function Callout(text: string, sub = "") {
    const main = Text(text, { role: "body", color: c.text, weight: "semibold", maxWidth: 4.6 });
    const content = sub ? VGroup(main, Text(sub, { role: "caption", color: c.muted, maxWidth: 4.6 })).arrange(DOWN, { buff: 0.12, alignedEdge: LEFT }) : main;
    const { group } = panel(content, { bar: c.accent });
    return Object.assign(group, {
      enter: (): Animation => FadeIn(group, { shift: [0.3, 0, 0] }),
    }) as Piece<{ enter(): Animation }>;
  }

  /**
   * A graph in the figure region. Lines are straight and drawn end to end inside the
   * ranges; the same function drawn again (dashed) lands on exactly the same points.
   */
  function Plot(o: { xRange: Point2; yRange: Point2; step?: number | Point2; xLabel?: string; yLabel?: string }) {
    const [sx, sy] = Array.isArray(o.step) ? o.step : [o.step ?? 1, o.step ?? 1];
    const ax = figure.axes({ xRange: [o.xRange[0], o.xRange[1], sx], yRange: [o.yRange[0], o.yRange[1], sy], numbers: true, xLabel: o.xLabel, yLabel: o.yLabel });
    const at = ([x, y]: Point2) => ax.c2p(x, y);
    const lines = new Map<string, { line: Mobject; from: Point2; to: Point2 }>();
    const fmt = (v: number) => String(Math.round(v * 100) / 100).replace("-", "−");
    let probe: { ring: Mobject; readout: Mobject; f?: Fn } | null = null;

    /** Where a straight line y = f(x) is inside the ranges: its two ends. */
    const ends = (f: Fn): [Point2, Point2] => {
      const [x0, x1] = o.xRange, [y0, y1] = o.yRange;
      const a = f(x0), slope = (f(x1) - a) / (x1 - x0);
      let lo = x0, hi = x1;
      if (Math.abs(slope) > 1e-9) {
        const xa = x0 + (y0 - a) / slope, xb = x0 + (y1 - a) / slope;
        lo = Math.max(lo, Math.min(xa, xb));
        hi = Math.min(hi, Math.max(xa, xb));
      }
      return [[lo, f(lo)], [hi, f(hi)]];
    };

    const plot = {
      axes: ax,
      /** Axes, numbers and axis titles. */
      show: (): Animation => AnimationGroup(Create(ax), FadeIn(ax.labels)),
      /** The point of graph coordinates (x, y). */
      at,
      /** Draw the line y = f(x) end to end. `dashed` draws it over the same line already there. */
      line(f: Fn, colour: string, opts: { dashed?: boolean } = {}): Animation {
        const key = f.toString().replace(/\s+/g, "");
        const same = lines.get(key);
        const [from, to] = same ? [same.from, same.to] : ends(f);
        const line = opts.dashed
          ? DashedLine(at(from), at(to), { color: colour, strokeWidth: 5, dashLength: 0.18, zIndex: 2 })
          : Line(at(from), at(to), { color: colour, strokeWidth: 5, zIndex: 1 });
        if (!same) lines.set(key, { line, from, to });
        return Create(line);
      },
      /** A dot that pops in, with an optional label beside it. */
      dot(p: Point2, colour: string, label = ""): Animation {
        const d = Dot(at(p), { color: colour, radius: 0.1, zIndex: 3 });
        const pop = GrowFromCenter(d, POP);
        if (!label) return pop;
        const l = MathTex(label, { color: colour, fontSize: 30 }).nextTo(d, UP, 0.15).shift([0.3, 0, 0]);
        return AnimationGroup(pop, FadeIn(l));
      },
      /** Where lines cross: a gold dot pulses twice, dashed guides drop to both axes, and gold numbers mark it there. */
      crossing(p: Point2, colour: string = c.accent): Animation {
        const d = Dot(at(p), { color: colour, radius: 0.11, zIndex: 4 });
        const ring = () => Circle(0.11, { color: colour, strokeWidth: 4 }).moveTo(d);
        const pulse = () => FadeOut(ring(), { scale: 3.2 });
        const base: Point2 = [Math.max(o.xRange[0], 0), Math.max(o.yRange[0], 0)];
        const gx = DashedLine(at(p), at([p[0], base[1]]), { color: colour, strokeWidth: 3, dashLength: 0.1 });
        const gy = DashedLine(at(p), at([base[0], p[1]]), { color: colour, strokeWidth: 3, dashLength: 0.1 });
        const tag = (text: string, where: Point2, dir: typeof DOWN) => MathTex(text, { color: colour, fontSize: 30 }).nextTo(at(where), dir, 0.18);
        const tags = [tag(fmt(p[0]), [p[0], base[1]], DOWN), tag(fmt(p[1]), [base[0], p[1]], LEFT)];
        // The axis numbers the gold ones replace step aside.
        const all = numbers(ax);
        const hide = tags
          .map((t) => all.reduce<Mobject | null>((best, n) => (dist(n.getCenter(), t.getCenter()) < (best ? dist(best.getCenter(), t.getCenter()) : 0.6) ? n : best), null))
          .filter((n): n is Mobject => n !== null);
        hidden.push(...hide);
        return Succession(
          GrowFromCenter(d, POP), pulse(), pulse(),
          AnimationGroup(Create(gx), Create(gy), ...hide.map((n) => n.animate.setOpacity(0)), ...tags.map((t) => FadeIn(t))),
        );
      },
      /** Rise over run on y = f(x), from x to x + run, with the slope written beside it. */
      slopeTriangle(f: Fn, x: number, run: number, label: string, colour: string = c.muted): Animation {
        const a = at([x, f(x)]), b = at([x + run, f(x)]), d = at([x + run, f(x + run)]);
        const tri = Polygon(a, b, d).setStroke(colour, 3).setFill(colour, 0.12);
        const t = MathTex(label, { color: colour, fontSize: 30 }).nextTo(VGroup(Line(b, d)), RIGHT, 0.15);
        return AnimationGroup(Create(tri), FadeIn(t));
      },
      /**
       * A ring slides along y = f(x) from x = from to x = to, with its coordinates read out
       * as it goes. `readout` writes them (Typst math).
       */
      probe(f: Fn, from: number, to: number, readout: (x: number, y: number) => string = (x, y) => `(${fmt(x)}, ${fmt(y)})`): Animation {
        const ring = Circle(0.17, { color: c.accent, strokeWidth: 4, zIndex: 5 }).moveTo(at([from, f(from)]));
        const text = alwaysRedraw(() => {
          const [x, y] = ax.p2c(ring.getCenter());
          return MathTex(readout(x, y), { color: c.text, fontSize: 28 }).nextTo(ring, UP, 0.12);
        });
        probe = { ring, readout: text, f };
        const path = Line(at([from, f(from)]), at([to, f(to)]));
        return Succession(AnimationGroup(FadeIn(ring), FadeIn(text)), MoveAlongPath(ring, path));
      },
      /** Move the probe ring off the line, to (x, y). */
      probeTo(p: Point2): Animation {
        return probe ? probe.ring.animate.moveTo(at(p)) : Wait(0.1);
      },
      /** Take the probe away. */
      probeOut(): Animation {
        const gone = probe;
        probe = null;
        return gone ? AnimationGroup(FadeOut(gone.ring), FadeOut(gone.readout)) : Wait(0.1);
      },
    };
    return plot;
  }

  /**
   * A full-width statement over the dimmed stage; `accent` is a word of `text` set in the
   * accent colour. enter() and exit().
   */
  function StatementCard(text: string, accent = "") {
    const safe = text.replace(/&/g, "&amp;").replace(/</g, "&lt;");
    const markup = accent ? safe.replace(accent, `<span color="${c.accent}">${accent}</span>`) : safe;
    const style = { role: "heading" as const, color: c.text, maxWidth: 8.8, align: "center" as const };
    const shown = accent ? MarkupText(markup, style) : Text(text, style);
    const { group } = panel(shown, { stroke: c.accent, padX: 0.6, padY: 0.45 });
    full.place(group);
    group.setZIndex(10);
    let stage: { m: Mobject; was: number }[] = [];
    /** What to dim: whole objects, except that ones holding hidden parts are dimmed part by part around them. */
    const pieces = (m: Mobject, was: number): { m: Mobject; was: number }[] => {
      const isHidden = (x: Mobject) => hidden.some((h) => same(h, x));
      if (isHidden(m)) return [];
      const holds = (x: Mobject): boolean => isHidden(x) || x.submobjects.some(holds);
      return holds(m) ? m.submobjects.flatMap((x) => pieces(x, was)) : [{ m, was }];
    };
    return Object.assign(group, {
      /** Dim everything else on screen to a shadow and bring the card in front. */
      enter(): Animation {
        const seen = scene.inspect();
        stage = scene.mobjects.flatMap((m, i) => (same(m, group) ? [] : pieces(m, seen[i] ? seen[i].opacity : 1)));
        // The card grows in at full strength once the stage has dimmed: text fading over text reads as a smudge.
        return Succession(AnimationGroup(...stage.map((s) => s.m.animate.setOpacity(DIM))), GrowFromCenter(group, POP));
      },
      /** Take the card away and bring the stage back. */
      exit(): Animation {
        const back = stage;
        stage = [];
        return Succession(group.animate.scale(0.02), FadeOut(group, { runTime: 0.01 }), AnimationGroup(...back.map((s) => s.m.animate.setOpacity(s.was))));
      },
    }) as Piece<{ enter(): Animation; exit(): Animation }>;
  }

  /** A red line through a mistake. */
  function StrikeOut(m: Mobject): Animation {
    const from = [m.getLeft()[0] - 0.05, m.getBottom()[1], 0], to = [m.getRight()[0] + 0.05, m.getTop()[1], 0];
    return Create(Line(from, to, { color: c.negative, strokeWidth: 6, zIndex: 6 }));
  }

  /** "Pause and think" with a pause sign, held for `seconds`. Play it without a runTime. */
  function PauseCard(seconds = 3, text = "Pause the video and think") {
    const bars = VGroup(
      RoundedRectangle({ width: 0.22, height: 0.75, cornerRadius: 0.06, fillColor: c.accent, fillOpacity: 1, strokeWidth: 0 }),
      RoundedRectangle({ width: 0.22, height: 0.75, cornerRadius: 0.06, fillColor: c.accent, fillOpacity: 1, strokeWidth: 0 }),
    ).arrange(RIGHT, { buff: 0.18 });
    const content = VGroup(bars, Text(text, { role: "heading", color: c.text })).arrange(RIGHT, { buff: 0.45 });
    const { group } = panel(content, { stroke: c.accent, padX: 0.6, padY: 0.4 });
    full.place(group);
    return Succession(FadeIn(group, { scale: 0.92 }), Wait(seconds), FadeOut(group));
  }

  /** A table whose rows come in one at a time: `this.play(table.next())`. The first row is the header. */
  function CaseTable(rows: string[][], region: Region = full) {
    const cells = rows.map((r, i) => r.map((t) => Text(t, { role: i === 0 ? "caption" : "body", color: i === 0 ? c.muted : c.text, weight: i === 0 ? "semibold" : "normal" })));
    const cols = Math.max(...rows.map((r) => r.length));
    const widths = Array.from({ length: cols }, (_, j) => Math.max(...cells.map((r) => (r[j] ? r[j].width : 0))));
    const lines = cells.map((r) => {
      let x = 0;
      for (let j = 0; j < r.length; j++) {
        r[j].moveTo([x + r[j].width / 2, 0, 0]);
        x += widths[j] + 0.6;
      }
      return VGroup(...r);
    });
    const table = VGroup(...lines).arrange(DOWN, { buff: 0.3, alignedEdge: LEFT });
    region.place(table, "top");
    lines.forEach((l) => region.adopt(l));
    let shown = 0;
    return Object.assign(table, {
      next(): Animation {
        const row = lines[shown++];
        return row ? FadeIn(row, { shift: [0.2, 0, 0] }) : Wait(0.1);
      },
    }) as Piece<{ next(): Animation }>;
  }

  return { c, EquationCard, Steps, AnswerBox, Callout, Plot, StatementCard, StrikeOut, PauseCard, CaseTable };
}
