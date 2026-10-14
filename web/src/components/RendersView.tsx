import { Link } from "@tanstack/react-router";
import {
  DownloadIcon,
  FolderOpenIcon,
  PlayCircleIcon,
  SquareIcon,
} from "lucide-react";
import { useState } from "react";
import { api, type Render } from "~/lib/api";
import { formatDuration, relativeTime } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { RenderLog } from "./panel/OutputPanel";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogBody,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "./ui/dialog";
import { Badge } from "./ui/input";
import { toast } from "./ui/toast";

export function RendersView() {
  const renders = useStudio((state) => state.renders);
  const [selected, setSelected] = useState<Render | null>(null);
  const shown = selected
    ? (renders.find((render) => render.id === selected.id) ?? selected)
    : null;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-[52px] shrink-0 items-center justify-between border-b px-6">
        <h1 className="text-sm font-medium">Render history</h1>
        <Button
          size="sm"
          variant="ghost-muted"
          onClick={() =>
            void api
              .openOutputs()
              .catch((error: Error) =>
                toast.error("Couldn't open folder", error.message),
              )
          }
        >
          <FolderOpenIcon /> Open outputs
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto max-w-5xl">
          {!renders.length ? (
            <div className="py-20 text-center text-sm text-muted-foreground">
              <PlayCircleIcon className="mx-auto mb-3 size-8" />
              Your rendered videos will appear here.
            </div>
          ) : (
            <div className="space-y-3">
              {renders.map((render) => (
                <article
                  key={render.id}
                  className="flex flex-wrap items-center gap-4 rounded-xl border bg-card p-4"
                >
                  <div className="min-w-0 flex-1">
                    <button
                      className="block max-w-full truncate text-left font-medium text-sm hover:underline"
                      onClick={() => setSelected(render)}
                    >
                      {render.name}
                    </button>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {render.scene} · {render.resolution || render.quality} ·{" "}
                      {relativeTime(render.created)}
                      {render.elapsed
                        ? ` · ${formatDuration(render.elapsed)}`
                        : ""}
                      {render.narrated ? " · Narrated" : ""}
                    </p>
                    {render.project && (
                      <Link
                        to="/projects/$projectId"
                        params={{ projectId: render.project }}
                        className="mt-1 block text-xs text-muted-foreground hover:underline"
                      >
                        {render.project_name || render.project}
                      </Link>
                    )}
                  </div>
                  <Badge
                    tone={
                      render.status === "completed"
                        ? "success"
                        : render.status === "failed"
                          ? "error"
                          : "neutral"
                    }
                  >
                    {render.stage || render.status}
                  </Badge>
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => setSelected(render)}
                  >
                    {render.video ? "View video" : "View log"}
                  </Button>
                  {render.video && (
                    <Button
                      size="icon-sm"
                      variant="ghost-muted"
                      aria-label={`Save ${render.name}`}
                      render={
                        <a href={`${render.video}?download=true`} download />
                      }
                    >
                      <DownloadIcon />
                    </Button>
                  )}
                  {["queued", "rendering"].includes(render.status) &&
                    render.can_cancel !== false && (
                      <Button
                        size="xs"
                        variant="outline"
                        onClick={() =>
                          void api
                            .cancelRender(render.id)
                            .catch((error: Error) =>
                              toast.error(
                                "Couldn't cancel render",
                                error.message,
                              ),
                            )
                        }
                      >
                        <SquareIcon /> Cancel
                      </Button>
                    )}
                </article>
              ))}
            </div>
          )}
        </div>
      </div>
      <Dialog
        open={!!shown}
        onOpenChange={(open) => !open && setSelected(null)}
      >
        <DialogPopup className="max-w-4xl">
          <DialogHeader>
            <DialogTitle>{shown?.name || "Render"}</DialogTitle>
          </DialogHeader>
          <DialogBody className="space-y-4">
            {shown?.video && (
              <video
                aria-label="Render history video"
                src={shown.video}
                controls
                className="max-h-[50vh] w-full rounded-lg bg-black"
              />
            )}
            {shown && (
              <div className="flex h-64 flex-col overflow-hidden rounded-lg border">
                <RenderLog render={shown} />
              </div>
            )}
          </DialogBody>
        </DialogPopup>
      </Dialog>
    </div>
  );
}
