import { Link } from "@tanstack/react-router";
import {
  ArrowDownIcon,
  ArrowUpIcon,
  DownloadIcon,
  FolderOpenIcon,
  PlayCircleIcon,
  SquareIcon,
} from "lucide-react";
import { useState } from "react";
import { api, type Render } from "~/lib/api";
import { formatDuration, relativeTime } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { QueueBar } from "./QueueBar";
import { RenderLog } from "./panel/OutputPanel";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogBody,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "./ui/dialog";
import { Badge, Spinner } from "./ui/input";
import { toast } from "./ui/toast";
import { WorkspaceBackButton } from "./WorkspaceBackButton";
import { RepairVideoButton } from "./VideoWorkflow";

export function RendersView() {
  const renders = useStudio((state) => state.renders);
  const videos = useStudio((state) => state.videos);
  const queue = useStudio((state) => state.queue);
  const waiting = queue?.waiting ?? [];
  const preparing = videos.filter(
    (video) => video.status === "checking" || video.status === "repairing",
  );
  const [selected, setSelected] = useState<Render | null>(null);
  const shown = selected
    ? (renders.find((render) => render.id === selected.id) ?? selected)
    : null;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex h-[52px] shrink-0 items-center justify-between gap-2 border-b px-4">
        <div className="flex min-w-0 items-center gap-2">
          <WorkspaceBackButton className="md:hidden" />
          <h1 className="truncate text-sm font-medium">Renders &amp; queue</h1>
          <span
            className="hidden text-xs text-muted-foreground sm:inline"
            role="status"
          >
            {renders.filter(
              (render) =>
                render.status === "queued" || render.status === "rendering",
            ).length + preparing.length || "No"}{" "}
            in queue
          </span>
        </div>
        <Button
          size="sm"
          variant="ghost-muted"
          aria-label="Open outputs"
          onClick={() =>
            void api
              .openOutputs()
              .catch((error: Error) =>
                toast.error("Couldn't open folder", error.message),
              )
          }
        >
          <FolderOpenIcon />{" "}
          <span className="hidden sm:inline">Open outputs</span>
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto max-w-5xl">
          <QueueBar />
          {preparing.map((video) => (
            <article
              key={video.id}
              className="mb-3 flex items-center gap-3 rounded-xl border bg-card p-4"
            >
              <Spinner className="size-4 shrink-0" />
              <div className="min-w-0 flex-1">
                <Link
                  to="/projects/$projectId"
                  params={{ projectId: video.project }}
                  search={{ thread: video.thread ?? undefined }}
                  className="block truncate text-sm font-medium hover:underline"
                >
                  {video.name}
                </Link>
                <p className="mt-1 text-xs text-muted-foreground">
                  {video.stage}
                </p>
              </div>
              <Button
                size="icon-sm"
                variant="ghost-muted"
                aria-label={`Stop video ${video.name}`}
                onClick={() =>
                  void api
                    .cancelVideo(video.id)
                    .catch((error: Error) =>
                      toast.error("Couldn't stop the video", error.message),
                    )
                }
              >
                <SquareIcon className="size-3 fill-current" />
              </Button>
            </article>
          ))}
          {!renders.length && !preparing.length ? (
            <div className="py-20 text-center text-sm text-muted-foreground">
              <PlayCircleIcon className="mx-auto mb-3 size-8" />
              Your rendered videos will appear here.
            </div>
          ) : (
            <div className="space-y-3">
              {[...renders]
                .sort((a, b) => {
                  const order = (item: Render) =>
                    item.status === "rendering"
                      ? 0
                      : item.status === "queued"
                        ? 1
                        : 2;
                  return (
                    order(a) - order(b) ||
                    (order(a) === 1
                      ? waiting.indexOf(a.id) - waiting.indexOf(b.id) ||
                        a.created.localeCompare(b.created)
                      : b.created.localeCompare(a.created))
                  );
                })
                .map((render) => (
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
                      {render.status === "queued" && waiting.includes(render.id)
                        ? `${queue?.paused ? "On hold" : "Waiting"} · #${waiting.indexOf(render.id) + 1}`
                        : render.stage || render.status}
                    </Badge>
                    {render.status === "queued" &&
                      waiting.includes(render.id) && (
                        <div className="flex">
                          <Button
                            size="icon-xs"
                            variant="ghost-muted"
                            aria-label={`Move ${render.name} up`}
                            disabled={waiting.indexOf(render.id) === 0}
                            onClick={() =>
                              void api
                                .moveRender(render.id, "up")
                                .catch((error: Error) =>
                                  toast.error(
                                    "Couldn't move the video",
                                    error.message,
                                  ),
                                )
                            }
                          >
                            <ArrowUpIcon />
                          </Button>
                          <Button
                            size="icon-xs"
                            variant="ghost-muted"
                            aria-label={`Move ${render.name} down`}
                            disabled={
                              waiting.indexOf(render.id) === waiting.length - 1
                            }
                            onClick={() =>
                              void api
                                .moveRender(render.id, "down")
                                .catch((error: Error) =>
                                  toast.error(
                                    "Couldn't move the video",
                                    error.message,
                                  ),
                                )
                            }
                          >
                            <ArrowDownIcon />
                          </Button>
                        </div>
                      )}
                    <Button
                      size="xs"
                      variant="outline"
                      onClick={() => setSelected(render)}
                    >
                      {render.video ? "View video" : "View log"}
                    </Button>
                    {render.status === "failed" && render.project && (
                      <RepairVideoButton
                        projectId={render.project}
                        renderId={render.id}
                        workspace={render.workspace ?? null}
                      />
                    )}
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
