import { Link } from "@tanstack/react-router";
import {
  ListOrderedIcon,
  PauseIcon,
  PlayIcon,
  SquareIcon,
  WrenchIcon,
} from "lucide-react";

import { api } from "~/lib/api";
import { videosLabel } from "~/lib/queue";
import { cn } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { Button } from "./ui/button";
import { Spinner } from "./ui/input";
import { Popover, PopoverPopup, PopoverTrigger } from "./ui/popover";
import { toast } from "./ui/toast";
import { Tooltip } from "./ui/tooltip";

function run(action: () => Promise<unknown>, message: string) {
  void action().catch((error: Error) => toast.error(message, error.message));
}

function Row({
  icon,
  name,
  detail,
  projectId,
  action,
}: {
  icon: React.ReactNode;
  name: string;
  detail: string;
  projectId?: string;
  action?: React.ReactNode;
}) {
  return (
    <li className="flex items-center gap-2.5 rounded-lg px-2 py-2 hover:bg-accent/50">
      <span className="grid size-6 shrink-0 place-items-center text-muted-foreground text-xs tabular-nums">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        {projectId ? (
          <Link
            to="/projects/$projectId"
            params={{ projectId }}
            className="block truncate font-medium text-sm hover:underline"
          >
            {name}
          </Link>
        ) : (
          <span className="block truncate font-medium text-sm">{name}</span>
        )}
        <span className="block truncate text-muted-foreground text-xs">
          {detail}
        </span>
      </span>
      {action}
    </li>
  );
}

/**
 * The render queue at a glance: what is rendering, being fixed and waiting,
 * in the order it will run, with the queue's start and pause controls.
 */
export function QueueButton() {
  const queue = useStudio((state) => state.queue);
  const renders = useStudio((state) => state.renders);
  const videos = useStudio((state) => state.videos);
  const rendering = renders.filter((render) => render.status === "rendering");
  const fixing = videos.filter(
    (video) => video.status === "checking" || video.status === "repairing",
  );
  const waiting = (queue?.waiting ?? [])
    .map((id) => renders.find((render) => render.id === id))
    .filter((render) => render !== undefined);
  const count = rendering.length + fixing.length + waiting.length;
  const held = !!queue?.paused && waiting.length > 0;

  return (
    <Popover>
      <Tooltip content="Render queue">
        <PopoverTrigger
          render={
            <Button
              size="sm"
              variant="ghost-muted"
              aria-label={
                count
                  ? `Queue ${count}, ${videosLabel(count)} in the render queue`
                  : "Queue, the render queue is empty"
              }
            />
          }
        >
          <ListOrderedIcon />
          <span className="hidden sm:inline">Queue</span>{" "}
          {count > 0 && (
            <span
              className={cn(
                "min-w-5 rounded-full px-1.5 font-medium text-2xs tabular-nums leading-5",
                held
                  ? "bg-amber-500/15 text-amber-700 dark:text-amber-300"
                  : "bg-primary/15 text-primary",
              )}
            >
              {count}
            </span>
          )}
        </PopoverTrigger>
      </Tooltip>
      <PopoverPopup
        side="bottom"
        align="end"
        className="w-[min(24rem,calc(100vw-1rem))] p-0"
      >
        <div className="flex items-center gap-2 border-b px-4 py-3">
          <div className="min-w-0 flex-1">
            <h2 className="font-medium text-sm">
              {held ? "Queue on hold" : "Render queue"}
            </h2>
            <p className="text-muted-foreground text-xs" role="status">
              {count
                ? [
                    rendering.length ? "1 rendering" : null,
                    fixing.length ? `${fixing.length} being fixed` : null,
                    waiting.length ? `${waiting.length} waiting` : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")
                : "Nothing is rendering or waiting."}
            </p>
          </div>
          {held ? (
            <Button
              size="xs"
              onClick={() =>
                run(() => api.startQueue(), "Couldn't start the queue")
              }
            >
              <PlayIcon className="fill-current" /> Start
            </Button>
          ) : (
            waiting.length > 0 && (
              <Button
                size="xs"
                variant="outline"
                onClick={() =>
                  run(() => api.pauseQueue(), "Couldn't pause the queue")
                }
              >
                <PauseIcon /> Pause
              </Button>
            )
          )}
        </div>
        {count ? (
          <ol
            className="max-h-80 overflow-y-auto p-2"
            aria-label="Queued videos"
          >
            {rendering.map((render) => (
              <Row
                key={render.id}
                icon={<Spinner className="size-4" />}
                name={render.name}
                detail={render.stage ?? "Rendering…"}
                projectId={render.project}
                action={
                  <Tooltip content="Stop render">
                    <Button
                      size="icon-xs"
                      variant="ghost-muted"
                      aria-label={`Stop rendering ${render.name}`}
                      onClick={() =>
                        run(
                          () => api.cancelRender(render.id),
                          "Couldn't stop the render",
                        )
                      }
                    >
                      <SquareIcon className="size-3 fill-current" />
                    </Button>
                  </Tooltip>
                }
              />
            ))}
            {fixing.map((video) => (
              <Row
                key={video.id}
                icon={<WrenchIcon className="size-4" />}
                name={video.name}
                detail={video.stage}
                projectId={video.project}
              />
            ))}
            {waiting.map((render, index) => (
              <Row
                key={render.id}
                icon={index + 1}
                name={render.name}
                detail={
                  held
                    ? "Waiting for the queue to start"
                    : index === 0 && !rendering.length
                      ? "Starting next"
                      : `Waiting · ${render.resolution || render.quality}`
                }
                projectId={render.project}
                action={
                  <Tooltip content="Remove from queue">
                    <Button
                      size="icon-xs"
                      variant="ghost-muted"
                      aria-label={`Remove ${render.name} from the queue`}
                      onClick={() =>
                        run(
                          () => api.cancelRender(render.id),
                          "Couldn't remove the video",
                        )
                      }
                    >
                      <SquareIcon className="size-3 fill-current" />
                    </Button>
                  </Tooltip>
                }
              />
            ))}
          </ol>
        ) : (
          <p className="px-4 py-6 text-center text-muted-foreground text-xs">
            Videos wait here when you render more than one, and start one at a
            time.
          </p>
        )}
        <div className="border-t px-4 py-2.5">
          <Link
            to="/renders"
            className="text-muted-foreground text-xs underline-offset-2 hover:text-foreground hover:underline"
          >
            Queue settings and all renders
          </Link>
        </div>
      </PopoverPopup>
    </Popover>
  );
}
