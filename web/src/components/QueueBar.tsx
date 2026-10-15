import { PauseIcon, PlayIcon, Trash2Icon } from "lucide-react";

import { api } from "~/lib/api";
import {
  QUEUE_ERROR_OPTIONS,
  QUEUE_START_OPTIONS,
  videosLabel,
} from "~/lib/queue";
import { useStudio } from "~/state/studio";
import { Button } from "./ui/button";
import { confirmAction } from "./ui/dialog";
import { Select } from "./ui/select";
import { toast } from "./ui/toast";

async function run(action: () => Promise<unknown>, message: string) {
  try {
    await action();
  } catch (error) {
    toast.error(message, (error as Error).message);
  }
}

/** The render queue's status and controls, shown while anything is queued. */
export function QueueBar() {
  const queue = useStudio((state) => state.queue);
  const agents = useStudio((state) => state.agents);
  const rendering = useStudio((state) =>
    state.renders.some((render) => render.status === "rendering"),
  );
  const fixing = useStudio(
    (state) =>
      state.videos.filter((video) => video.status === "repairing").length,
  );
  const waiting = queue?.waiting.length ?? 0;
  if (!queue || (!waiting && !rendering && !fixing)) return null;

  const settings = agents?.settings;
  const save = (changes: Parameters<typeof api.saveAgentSettings>[0]) =>
    void run(async () => {
      await api.saveAgentSettings(changes);
      await useStudio.getState().loadAgents();
    }, "Couldn't save the queue settings");

  const summary = [
    rendering ? "Rendering now" : null,
    waiting ? `${videosLabel(waiting)} waiting` : null,
    fixing ? `${videosLabel(fixing)} being fixed` : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <section
      aria-label="Render queue"
      className="mb-4 rounded-xl border bg-card p-4"
    >
      <div className="flex flex-wrap items-center gap-3">
        <div className="min-w-0 flex-1">
          <h2 className="font-medium text-sm">
            {queue.paused && waiting ? "Queue on hold" : "Render queue"}
          </h2>
          <p className="mt-0.5 text-muted-foreground text-xs" role="status">
            {summary}
            {queue.paused && waiting
              ? queue.start === "manual"
                ? " · Press Start queue to render them one at a time."
                : " · Paused. The current video finishes first."
              : ""}
          </p>
        </div>
        <div className="flex items-center gap-2">
          {queue.paused && waiting > 0 ? (
            <Button
              size="sm"
              onClick={() =>
                void run(() => api.startQueue(), "Couldn't start the queue")
              }
            >
              <PlayIcon className="fill-current" /> Start queue
            </Button>
          ) : (
            waiting > 0 && (
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  void run(() => api.pauseQueue(), "Couldn't pause the queue")
                }
              >
                <PauseIcon /> Pause
              </Button>
            )
          )}
          {waiting > 0 && (
            <Button
              size="sm"
              variant="ghost-muted"
              onClick={() =>
                void (async () => {
                  const ok = await confirmAction({
                    title: "Clear the waiting videos?",
                    description: `${videosLabel(waiting)} will be removed from the queue. The video that is rendering keeps going.`,
                    confirm: "Clear queue",
                    destructive: true,
                  });
                  if (ok)
                    await run(
                      () => api.clearQueue(),
                      "Couldn't clear the queue",
                    );
                })()
              }
            >
              <Trash2Icon /> Clear waiting
            </Button>
          )}
        </div>
      </div>
      {settings && (
        <div className="mt-3 flex flex-wrap items-center gap-x-5 gap-y-2 border-t pt-3 text-muted-foreground text-xs">
          <label className="flex items-center gap-2">
            Start
            <Select
              aria-label="Start rendering"
              size="sm"
              value={settings.queue_start}
              options={QUEUE_START_OPTIONS}
              onChange={(queue_start) => save({ queue_start })}
            />
          </label>
          <label className="flex items-center gap-2">
            If a video fails
            <Select
              aria-label="If a video fails"
              size="sm"
              value={settings.queue_on_error}
              options={QUEUE_ERROR_OPTIONS}
              onChange={(queue_on_error) => save({ queue_on_error })}
            />
          </label>
        </div>
      )}
    </section>
  );
}
