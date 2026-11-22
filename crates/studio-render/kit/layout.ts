// Studio layout v1. Studio keeps this file up to date; change scenes, not this file.
//
// Content goes on screen through four regions of the 14.2 × 8 frame, never at
// coordinates. Studio checks after every play that each thing stays inside its region,
// and that text placed some other way sits inside one; anything else is a layout problem.
import { Axes, AnimationGroup, FadeOut, Text, VGroup, Wait, DOWN, UP, LEFT, type Animation, type AxesOptions, type AxesMobject, type Mobject } from "manim";

/** A region's edges in scene units. */
export interface Box { left: number; right: number; top: number; bottom: number }

const BOXES: Record<string, Box> = {
  kicker: { left: -6.2, right: -1.3, top: 3.7, bottom: 2.9 },
  work: { left: -6.2, right: -1.3, top: 2.8, bottom: -3.4 },
  figure: { left: -0.45, right: 6.5, top: 3.2, bottom: -3.3 },
  full: { left: -5.0, right: 5.0, top: 3.4, bottom: -3.4 },
};

/** Space between items stacked in a region. */
const GAP = 0.3;

interface Placed { m: Mobject; region: string }
const LAYOUT = ((globalThis as any).__LAYOUT ??= { boxes: BOXES, items: [] as Placed[], stacks: {} }) as {
  boxes: Record<string, Box>;
  items: Placed[];
  /** What each region's push() has stacked, for the check's advice. */
  stacks: Record<string, Mobject[]>;
};

function track(m: Mobject, region: string) {
  LAYOUT.items = LAYOUT.items.filter((p) => p.m !== m);
  LAYOUT.items.push({ m, region });
}

/** Shrink `m` (never grow it) so it fits `width` × `height`. Axes are never scaled: they are made to size. */
function fit(m: Mobject, width: number, height: number) {
  if (m.kind === "Axes") return;
  const k = Math.min(1, width / Math.max(m.width, 1e-6), height / Math.max(m.height, 1e-6));
  if (k < 1) m.scale(k);
}

export class Region {
  readonly name: string;
  readonly box: Box;
  /** What push() has stacked, top to bottom. */
  items: Mobject[] = [];

  constructor(name: string) {
    this.name = name;
    this.box = BOXES[name];
    LAYOUT.stacks[name] = this.items;
  }

  get width() { return this.box.right - this.box.left; }
  get height() { return this.box.top - this.box.bottom; }

  /** Put `m` in the region, shrunk to fit: centred, or against its top or left edge. Returns `m`. */
  place<M extends Mobject>(m: M, where: "center" | "top" | "left" = "center"): M {
    const b = this.box;
    fit(m, this.width, this.height);
    const x = where === "left" ? b.left + m.width / 2 : (b.left + b.right) / 2;
    const y = where === "top" ? b.top - m.height / 2 : (b.top + b.bottom) / 2;
    m.moveTo([x, y, 0]);
    track(m, this.name);
    return m;
  }

  /** Put `m` below the last pushed item (or at the top), left-aligned, shrunk to the region's width. Returns `m`. */
  push<M extends Mobject>(m: M, gap = GAP): M {
    const b = this.box;
    fit(m, this.width, this.height);
    const last = this.items[this.items.length - 1];
    const top = last ? last.getBottom()[1] - gap : b.top;
    m.moveTo([b.left + m.width / 2, top - m.height / 2, 0]);
    this.items.push(m);
    track(m, this.name);
    return m;
  }

  /** Fade out everything push() stacked and start again at the top: `this.play(work.clear())`. */
  clear(): Animation {
    const gone = this.items;
    this.items = [];
    LAYOUT.stacks[this.name] = this.items;
    LAYOUT.items = LAYOUT.items.filter((p) => !gone.includes(p.m));
    return gone.length ? AnimationGroup(...gone.map((m) => FadeOut(m))) : Wait(0.1);
  }

  /**
   * Axes made to fill the region, with room for their numbers and titles. `xLabel` goes
   * under the axes and `yLabel` above them; both are in `ax.labels`, to show with the axes.
   */
  axes(o: AxesOptions & { xLabel?: string; yLabel?: string } = {}): AxesMobject & { labels: Mobject } {
    const pad = 0.95;
    const { xLabel, yLabel, ...rest } = o;
    const ax = Axes({ ...rest, xLength: rest.xLength ?? this.width - 2 * pad, yLength: rest.yLength ?? this.height - 2 * pad }) as AxesMobject & { labels: Mobject };
    this.place(ax);
    const titles: Mobject[] = [];
    if (xLabel) titles.push(Text(xLabel, { role: "caption" }).nextTo(ax, DOWN, 0.12));
    if (yLabel) titles.push(Text(yLabel, { role: "caption" }).nextTo(ax, UP, 0.12).alignTo(ax, LEFT));
    ax.labels = VGroup(...titles);
    return ax;
  }
}

/** Small label above the work column; heading() goes here. */
export const kicker = new Region("kicker");
/** Equations, worked steps and answers, stacked with push(). */
export const work = new Region("work");
/** The graph or diagram panel on the right. */
export const figure = new Region("figure");
/** A full-width statement card over the stage. */
export const full = new Region("full");

/** The scene's title, in the kicker region. */
export function heading(text: string): Mobject {
  return kicker.place(Text(text, { role: "heading" }), "left");
}
