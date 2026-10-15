import { Link } from "@tanstack/react-router";
import { ArrowLeftIcon } from "lucide-react";

import { cn } from "~/lib/utils";
import { Button } from "./ui/button";
import { Tooltip } from "./ui/tooltip";

/** One return action for secondary workspace pages, in the footer or mobile header. */
export function WorkspaceBackButton({
  compact = false,
  className,
}: {
  compact?: boolean;
  className?: string;
}) {
  return (
    <Tooltip content={compact ? "Back to workspace" : null} side="right">
      <Button
        variant="ghost-muted"
        size="sm"
        aria-label="Back"
        data-workspace-back=""
        className={cn(
          "min-w-0 justify-start gap-2 bg-secondary px-2.5 text-foreground",
          compact && "size-9 flex-none justify-center p-0",
          className,
        )}
        render={<Link to="/" />}
      >
        <ArrowLeftIcon />
        {!compact && <span>Back</span>}
      </Button>
    </Tooltip>
  );
}
