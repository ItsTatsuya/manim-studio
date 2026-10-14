import type * as React from "react";

import { cn } from "~/lib/utils";

export const fieldClass =
  "w-full min-w-0 rounded-[var(--control-radius)] border border-input bg-background px-2.5 text-sm text-foreground shadow-xs/5 outline-none transition-[box-shadow,border-color] placeholder:text-muted-foreground/70 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/20 disabled:opacity-50 aria-invalid:border-destructive/60 dark:bg-input/30";

export function Input({ className, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      className={cn(fieldClass, "h-8", className)}
      data-slot="input"
      {...props}
    />
  );
}

export function Textarea({
  className,
  ...props
}: React.ComponentProps<"textarea">) {
  return (
    <textarea
      className={cn(fieldClass, "min-h-20 py-2 leading-relaxed", className)}
      data-slot="textarea"
      {...props}
    />
  );
}

export function Label({ className, ...props }: React.ComponentProps<"label">) {
  return (
    <label
      className={cn("font-medium text-[13px] text-foreground", className)}
      {...props}
    />
  );
}

export function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center gap-0.5 rounded border border-border bg-muted px-1 font-sans text-[11px] text-muted-foreground",
        className,
      )}
      {...props}
    />
  );
}

export function Separator({
  className,
  vertical,
  ...props
}: React.ComponentProps<"div"> & { vertical?: boolean }) {
  return (
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      className={cn(
        "shrink-0 bg-border",
        vertical ? "w-px self-stretch" : "h-px w-full",
        className,
      )}
      {...props}
    />
  );
}

export function Spinner({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4 animate-spin", className)}
      viewBox="0 0 16 16"
      fill="none"
      aria-hidden
    >
      <circle
        cx="8"
        cy="8"
        r="6.5"
        stroke="currentColor"
        strokeOpacity="0.2"
        strokeWidth="2"
      />
      <path
        d="M14.5 8A6.5 6.5 0 0 0 8 1.5"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
    </svg>
  );
}

const badgeTones = {
  neutral: "border-border bg-muted text-muted-foreground",
  success: "border-success/25 bg-success/10 text-success-foreground",
  warning: "border-warning/30 bg-warning/10 text-warning-foreground",
  error: "border-destructive/30 bg-destructive/10 text-destructive-foreground",
  info: "border-primary/30 bg-primary/10 text-info-foreground",
};

export function Badge({
  className,
  tone = "neutral",
  ...props
}: React.ComponentProps<"span"> & { tone?: keyof typeof badgeTones }) {
  return (
    <span
      className={cn(
        "inline-flex h-5 items-center gap-1 whitespace-nowrap rounded-full border px-2 font-medium text-[11px] [&_svg]:size-3",
        badgeTones[tone],
        className,
      )}
      {...props}
    />
  );
}

const dotColors = {
  success: "bg-success",
  warning: "bg-warning",
  error: "bg-destructive",
  neutral: "bg-muted-foreground/50",
  info: "bg-primary",
};

export function StatusDot({
  tone,
  pulse,
}: {
  tone: keyof typeof dotColors;
  pulse?: boolean;
}) {
  return (
    <span className="relative inline-flex size-2 shrink-0">
      {pulse && (
        <span
          className={cn(
            "absolute inset-0 animate-ping rounded-full opacity-60",
            dotColors[tone],
          )}
        />
      )}
      <span
        className={cn(
          "relative inline-flex size-2 rounded-full",
          dotColors[tone],
        )}
      />
    </span>
  );
}
