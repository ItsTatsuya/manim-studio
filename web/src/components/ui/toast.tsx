import {
  CircleAlertIcon,
  CircleCheckIcon,
  InfoIcon,
  XIcon,
} from "lucide-react";
import type * as React from "react";
import { create } from "zustand";

import { cn } from "~/lib/utils";

type Tone = "info" | "success" | "error";

interface ToastItem {
  id: number;
  tone: Tone;
  title: string;
  description?: string;
  action?: { label: string; onClick: () => void };
}

const useToasts = create<{ toasts: ToastItem[] }>(() => ({ toasts: [] }));
let nextId = 1;

export function toast(
  title: string,
  options: {
    tone?: Tone;
    description?: string;
    action?: ToastItem["action"];
    duration?: number;
  } = {},
) {
  const id = nextId++;
  const item: ToastItem = {
    id,
    tone: options.tone ?? "info",
    title,
    description: options.description,
    action: options.action,
  };
  useToasts.setState((state) => ({
    toasts: [...state.toasts.slice(-3), item],
  }));
  durations.set(id, options.duration ?? (item.tone === "error" ? 8000 : 4000));
  schedule(id);
  return id;
}

const durations = new Map<number, number>();
const timers = new Map<number, number>();

function schedule(id: number) {
  window.clearTimeout(timers.get(id));
  timers.set(
    id,
    window.setTimeout(() => dismiss(id), durations.get(id) ?? 4000),
  );
}

/** Hovering or focusing a toast keeps it on screen so it can be read. */
function hold(id: number) {
  window.clearTimeout(timers.get(id));
  timers.delete(id);
}

toast.error = (title: string, description?: string) =>
  toast(title, { tone: "error", description });
toast.success = (title: string, description?: string) =>
  toast(title, { tone: "success", description });

function dismiss(id: number) {
  hold(id);
  durations.delete(id);
  useToasts.setState((state) => ({
    toasts: state.toasts.filter((item) => item.id !== id),
  }));
}

const icons: Record<Tone, React.ReactNode> = {
  info: <InfoIcon className="size-4 text-info-foreground" />,
  success: <CircleCheckIcon className="size-4 text-success-foreground" />,
  error: <CircleAlertIcon className="size-4 text-destructive-foreground" />,
};

export function Toaster() {
  const toasts = useToasts((state) => state.toasts);
  return (
    <div
      aria-live="polite"
      className="pointer-events-none fixed end-4 bottom-4 z-[300] flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2"
    >
      {toasts.map((item) => (
        <div
          key={item.id}
          role={item.tone === "error" ? "alert" : "status"}
          onPointerEnter={() => hold(item.id)}
          onPointerLeave={() => schedule(item.id)}
          onFocus={() => hold(item.id)}
          onBlur={() => schedule(item.id)}
          className={cn(
            "pointer-events-auto flex animate-[toast-in_160ms_var(--ease-drawer)] items-start gap-2.5 rounded-xl border bg-popover p-3 text-popover-foreground shadow-popup",
          )}
        >
          <div className="mt-0.5">{icons[item.tone]}</div>
          <div className="min-w-0 flex-1">
            <div className="font-medium text-ui">{item.title}</div>
            {item.description && (
              <div className="mt-0.5 max-h-40 overflow-y-auto break-words text-muted-foreground text-xs">
                {item.description}
              </div>
            )}
            {item.action && (
              <button
                type="button"
                className="mt-1.5 font-medium text-info-foreground text-xs hover:underline"
                onClick={() => {
                  item.action?.onClick();
                  dismiss(item.id);
                }}
              >
                {item.action.label}
              </button>
            )}
          </div>
          <button
            type="button"
            aria-label="Dismiss"
            className="relative rounded p-0.5 text-muted-foreground after:absolute after:-inset-2 after:content-[''] hover:bg-accent hover:text-foreground"
            onClick={() => dismiss(item.id)}
          >
            <XIcon className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}
