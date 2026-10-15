import type * as React from "react";

import { cn } from "~/lib/utils";

/** A native progress bar, styled once so every download and update agrees. */
export function Progress({
  className,
  ...props
}: React.ComponentProps<"progress">) {
  return (
    <progress
      className={cn(
        "h-1.5 w-full appearance-none overflow-hidden rounded-full [&::-moz-progress-bar]:bg-primary [&::-webkit-progress-bar]:bg-muted-foreground/20 [&::-webkit-progress-value]:rounded-full [&::-webkit-progress-value]:bg-primary [&::-webkit-progress-value]:transition-[width] [&::-webkit-progress-value]:duration-300",
        className,
      )}
      {...props}
    />
  );
}
