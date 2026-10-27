import type { HouseStyle } from "~/lib/api";
import { cn } from "~/lib/utils";

function family(name: string, fallback: string) {
  return name ? `"${name}", ${fallback}` : fallback;
}

/**
 * A 16:9 thumbnail of a slide in this style: background, title placement,
 * fonts, a key-idea box, an example box and the palette.
 */
export function StyleSwatch({
  style,
  className,
  detailed = false,
}: {
  style: HouseStyle;
  className?: string;
  /** Also show a heading, bullets and an equation line, like a full slide. */
  detailed?: boolean;
}) {
  const { colors, fonts, sizes, layout } = style;
  // Manim's frame is 8 units tall; the preview scales font sizes to its height.
  const scale = (size: number) => `${(size / 48) * 7}cqh`;
  const margin = `${(layout.margin / 8) * 100}cqh`;
  return (
    <div
      role="img"
      aria-label={`${style.name} preview`}
      className={cn(
        "relative aspect-video w-full overflow-hidden rounded-lg border [container-type:size]",
        className,
      )}
      style={{ background: colors.background, color: colors.text }}
    >
      <div
        className="absolute flex flex-col"
        style={{
          inset: margin,
          alignItems: layout.title === "top-left" ? "flex-start" : "center",
          justifyContent: layout.title === "center" ? "center" : "flex-start",
        }}
      >
        <div
          className="truncate font-bold leading-tight"
          style={{
            fontFamily: family(fonts.heading, "system-ui, sans-serif"),
            fontSize: scale(sizes.title),
            maxWidth: "100%",
          }}
        >
          Learning objectives
        </div>
        <div
          className="mt-[1.5cqh] h-[0.8cqh] w-[18cqh] rounded-full"
          style={{ background: colors.primary }}
        />
        {detailed && layout.title !== "center" && (
          <div
            className="mt-[3cqh] flex w-full flex-col gap-[1cqh] leading-snug"
            style={{
              fontFamily: family(fonts.body, "system-ui, sans-serif"),
              fontSize: scale(sizes.body),
            }}
          >
            <div
              className="truncate"
              style={{
                color: colors.primary,
                fontFamily: family(fonts.heading, "system-ui, sans-serif"),
                fontSize: scale(sizes.heading),
              }}
            >
              Why it works
            </div>
            <div className="truncate">• A small change passes through both</div>
            <div className="truncate">
              • Multiply the rates:{" "}
              <span style={{ color: colors.primary }}>f′(g(x))</span> ·{" "}
              <span style={{ color: colors.secondary }}>g′(x)</span>
            </div>
          </div>
        )}
        {layout.title !== "center" && (
          <div
            className={cn(
              "grid w-full grid-cols-2 gap-[3cqh]",
              detailed ? "mt-[3cqh]" : "mt-[4cqh]",
            )}
            style={{
              fontFamily: family(fonts.body, "system-ui, sans-serif"),
              fontSize: scale(sizes.body),
            }}
          >
            <div
              className="rounded-[1.5cqh] border-[0.5cqh] px-[2cqh] py-[1.5cqh] leading-snug"
              style={{
                background: colors.key_fill,
                borderColor: colors.primary,
              }}
            >
              <div
                style={{
                  color: colors.primary,
                  fontSize: scale(sizes.small),
                  fontFamily: family(fonts.heading, "system-ui, sans-serif"),
                }}
              >
                Key idea
              </div>
              a² + b² = c²
            </div>
            <div
              className="rounded-[1.5cqh] border-[0.5cqh] px-[2cqh] py-[1.5cqh] leading-snug"
              style={{
                background: colors.example_fill,
                borderColor: colors.secondary,
              }}
            >
              <div
                style={{
                  color: colors.secondary,
                  fontSize: scale(sizes.small),
                  fontFamily: family(fonts.heading, "system-ui, sans-serif"),
                }}
              >
                Example
              </div>
              3, 4,{" "}
              <span style={{ color: colors.accent, fontWeight: 600 }}>5</span>
            </div>
            <div
              className="col-span-2 truncate"
              style={{
                color: colors.muted,
                fontSize: scale(sizes.small),
                fontFamily: family(fonts.code, "ui-monospace, monospace"),
              }}
            >
              <span style={{ color: colors.positive }}>✓ right angle</span>
              {"  "}
              <span style={{ color: colors.negative }}>✗ obtuse</span>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

/** The palette as a row of small chips. */
export function StyleChips({ style }: { style: HouseStyle }) {
  const keys = [
    "background",
    "text",
    "primary",
    "secondary",
    "accent",
    "positive",
    "negative",
  ] as const;
  return (
    <span className="inline-flex items-center gap-1" aria-hidden>
      {keys.map((key) => (
        <span
          key={key}
          className="size-3 rounded-full border border-black/10 dark:border-white/15"
          style={{ background: style.colors[key] }}
        />
      ))}
    </span>
  );
}
