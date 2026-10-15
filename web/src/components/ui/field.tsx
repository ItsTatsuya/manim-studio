import type * as React from "react";

import { cn } from "~/lib/utils";

/** A labelled form control: one label style and one label-to-control gap. */
export function Field({
  label,
  className,
  children,
}: {
  label: React.ReactNode;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <label
      className={cn(
        "flex min-w-0 flex-col gap-1.5 text-muted-foreground text-xs",
        className,
      )}
    >
      {label}
      {children}
    </label>
  );
}
