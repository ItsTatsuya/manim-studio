import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";
import { XIcon } from "lucide-react";
import type * as React from "react";
import { create } from "zustand";

import { cn } from "~/lib/utils";
import { Button } from "./button";

export const Dialog = DialogPrimitive.Root;

export function DialogPopup({
  className,
  children,
  showClose = true,
  ...props
}: DialogPrimitive.Popup.Props & { showClose?: boolean }) {
  return (
    <DialogPrimitive.Portal>
      <DialogPrimitive.Backdrop className="fixed inset-0 z-50 bg-black/40 backdrop-blur-[2px] transition-opacity duration-150 data-[ending-style]:opacity-0 data-[starting-style]:opacity-0 dark:bg-black/60" />
      <div className="fixed inset-0 z-50 grid place-items-center p-4">
        <DialogPrimitive.Popup
          className={cn(
            "relative flex max-h-full w-full max-w-lg flex-col rounded-2xl border bg-popover text-popover-foreground shadow-2xl outline-none transition-[opacity,scale] duration-150 data-[ending-style]:scale-[0.98] data-[starting-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[starting-style]:opacity-0",
            className,
          )}
          {...props}
        >
          {children}
          {showClose && (
            <DialogPrimitive.Close
              aria-label="Close"
              className="absolute end-2.5 top-2.5"
              render={<Button size="icon-sm" variant="ghost-muted" />}
            >
              <XIcon />
            </DialogPrimitive.Close>
          )}
        </DialogPrimitive.Popup>
      </div>
    </DialogPrimitive.Portal>
  );
}

export function DialogHeader({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("flex flex-col gap-1.5 px-5 pt-5 pb-3", className)}
      {...props}
    />
  );
}

export function DialogTitle({
  className,
  ...props
}: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      className={cn("pe-8 font-semibold text-base", className)}
      {...props}
    />
  );
}

export function DialogDescription({
  className,
  ...props
}: DialogPrimitive.Description.Props) {
  return (
    <DialogPrimitive.Description
      className={cn("text-muted-foreground text-sm", className)}
      {...props}
    />
  );
}

export function DialogBody({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("min-h-0 overflow-y-auto px-5 pb-4", className)}
      {...props}
    />
  );
}

export function DialogFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "flex justify-end gap-2 rounded-b-2xl border-t bg-muted/60 px-5 py-3",
        className,
      )}
      {...props}
    />
  );
}

interface ConfirmRequest {
  title: string;
  description?: React.ReactNode;
  confirm?: string;
  destructive?: boolean;
  resolve: (value: boolean) => void;
}

const useConfirmStore = create<{ request: ConfirmRequest | null }>(() => ({
  request: null,
}));

/** Ask before a destructive action. Resolves true when confirmed. */
export function confirmAction(options: Omit<ConfirmRequest, "resolve">) {
  return new Promise<boolean>((resolve) => {
    useConfirmStore.getState().request?.resolve(false);
    useConfirmStore.setState({ request: { ...options, resolve } });
  });
}

export function ConfirmDialogHost() {
  const request = useConfirmStore((state) => state.request);
  const close = (value: boolean) => {
    request?.resolve(value);
    useConfirmStore.setState({ request: null });
  };
  return (
    <Dialog open={!!request} onOpenChange={(open) => !open && close(false)}>
      <DialogPopup className="max-w-md" showClose={false} role="alertdialog">
        <DialogHeader>
          <DialogTitle>{request?.title}</DialogTitle>
          {request?.description && (
            <DialogDescription>{request.description}</DialogDescription>
          )}
        </DialogHeader>
        <DialogFooter>
          <Button variant="ghost" onClick={() => close(false)} autoFocus>
            Cancel
          </Button>
          <Button
            variant={request?.destructive ? "destructive" : "default"}
            onClick={() => close(true)}
          >
            {request?.confirm ?? "Continue"}
          </Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}
