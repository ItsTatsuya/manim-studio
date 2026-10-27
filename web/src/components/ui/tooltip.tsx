import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";
import type * as React from "react";

import { cn } from "~/lib/utils";

export const TooltipProvider = TooltipPrimitive.Provider;

/** A hover hint rendered on the child element. */
export function Tooltip({
  content,
  children,
  side = "bottom",
  className,
}: {
  content: React.ReactNode;
  children: React.ReactElement;
  side?: "top" | "bottom" | "left" | "right";
  className?: string;
}) {
  if (!content) return children;
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger render={children} />
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Positioner
          side={side}
          sideOffset={6}
          className="z-[200]"
        >
          <TooltipPrimitive.Popup
            className={cn(
              "max-w-72 rounded-md border bg-popover px-2 py-1 text-popover-foreground text-xs shadow-md transition-opacity duration-100 ease-out data-[ending-style]:opacity-0 data-[starting-style]:opacity-0",
              className,
            )}
          >
            {content}
          </TooltipPrimitive.Popup>
        </TooltipPrimitive.Positioner>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}
