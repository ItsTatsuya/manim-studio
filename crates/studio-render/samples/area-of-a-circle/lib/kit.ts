// Shared helpers for this lesson's scenes. Colours come from the house style the
// studio applies (this.theme), so the same scenes work in every house style.
import { Text, RoundedRectangle, VGroup, UL, type Mobject, type Theme } from "manim";

export interface Palette {
  background: string; text: string; muted: string; primary: string; secondary: string;
  accent: string; positive: string; negative: string; key_fill: string; example_fill: string;
}

const PAPER: Palette = {
  background: "#FBFAF6", text: "#1E2731", muted: "#68717D", primary: "#3B5BDB", secondary: "#0C8599",
  accent: "#E8590C", positive: "#2B8A3E", negative: "#C92A2A", key_fill: "#EDF1FF", example_fill: "#E3F7FA",
};

export function palette(theme: Theme | null): Palette {
  return { ...PAPER, ...(theme?.colors ?? {}) } as Palette;
}

/** A slide title in the top-left corner. */
export function heading(text: string): Mobject {
  return Text(text, { role: "title" }).toCorner(UL, 0.65);
}

/** A rounded card behind `content`. */
export function card(content: Mobject, fill: string, stroke: string, pad = 0.35): Mobject {
  const box = RoundedRectangle({
    width: content.width + 2 * pad,
    height: content.height + 2 * pad,
    cornerRadius: 0.18,
    fillColor: fill,
    fillOpacity: 1,
    strokeColor: stroke,
    strokeWidth: 3,
  }).moveTo(content);
  return VGroup(box, content);
}
