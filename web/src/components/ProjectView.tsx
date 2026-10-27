import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  PanelRightCloseIcon,
  PanelRightOpenIcon,
  PlayIcon,
  SquareIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api } from "~/lib/api";
import { activeRender, renderProject, renderProblem } from "~/lib/render";
import { cn } from "~/lib/utils";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { ChatPane } from "./chat/ChatPane";
import { RightPanel, type PanelTab } from "./panel/RightPanel";
import { QueueButton } from "./QueueButton";
import { Button } from "./ui/button";
import { Kbd, Spinner } from "./ui/input";
import { toast } from "./ui/toast";
import { Tooltip } from "./ui/tooltip";
import { useVideoWorkflow, videoIsActive } from "./VideoWorkflow";

interface Layout {
  width: number;
  open: boolean;
  tab: PanelTab;
}

const DEFAULT_LAYOUT: Layout = { width: 620, open: true, tab: "preview" };

function useLayout() {
  const raw = useStudio((state) => state.prefs["studio-layout"]);
  const layout = useMemo<Layout>(() => {
    try {
      return { ...DEFAULT_LAYOUT, ...JSON.parse(raw || "{}") };
    } catch {
      return DEFAULT_LAYOUT;
    }
  }, [raw]);
  const update = useCallback(
    (changes: Partial<Layout>) =>
      useStudio
        .getState()
        .setPref("studio-layout", JSON.stringify({ ...layout, ...changes })),
    [layout],
  );
  return [layout, update] as const;
}

export function ProjectView() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { thread: threadId } = useSearch({ from: "/projects/$projectId" });
  const navigate = useNavigate();
  const project = useStudio((state) =>
    state.projects.find((item) => item.id === projectId),
  );
  const projectsLoaded = useStudio((state) => state.projects.length > 0);
  const version = useStudio((state) => state.projectVersions[projectId] ?? 0);
  const renders = useStudio((state) => state.renders);
  const health = useStudio((state) => state.health);
  const analysis = useProjectDoc((state) => state.analysis);
  const thread = useStudio((state) =>
    threadId
      ? state.threads[projectId]?.find((t) => t.id === threadId)
      : undefined,
  );
  const workspaceId = thread?.workspace ?? null;
  const loaded = useProjectDoc(
    (state) =>
      state.project?.id === projectId && state.workspace === workspaceId,
  );
  const [layout, setLayout] = useLayout();
  const workspace = useRef<HTMLDivElement>(null);
  const [availableWidth, setAvailableWidth] = useState(1200);
  useEffect(() => {
    const element = workspace.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) =>
      setAvailableWidth(entry.contentRect.width),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const panelWidth = Math.max(0, Math.min(layout.width, availableWidth - 380));
  const running = activeRender(renders);
  const localRender = activeRender(
    renders.filter(
      (render) =>
        render.project === projectId &&
        (render.workspace ?? null) === workspaceId,
    ),
  );
  const renderingHere = !!localRender;
  const video = useVideoWorkflow(projectId);
  const makingVideo = videoIsActive(video);
  const problem = renderProblem(loaded ? analysis : null, health);

  useEffect(() => {
    void useProjectDoc
      .getState()
      .open(projectId, workspaceId)
      .catch((error: Error) =>
        toast.error("Couldn't open the project", error.message),
      );
    useStudio.getState().setPref("manim-project", projectId);
    void useStudio
      .getState()
      .loadThreads(projectId)
      .catch(() => undefined);
  }, [projectId, workspaceId]);

  // Files changed on disk (an agent edited them): show the new version.
  const firstVersion = useRef(version);
  useEffect(() => {
    if (version !== firstVersion.current) {
      firstVersion.current = version;
      void useProjectDoc
        .getState()
        .reload()
        .catch((error: Error) =>
          toast.error("Couldn't reload project", error.message),
        );
    }
  }, [version]);

  useEffect(() => {
    if (projectsLoaded && !project) void navigate({ to: "/", replace: true });
  }, [projectsLoaded, project, navigate]);

  const render = useCallback(async () => {
    try {
      await renderProject(projectId, {
        workspace: workspaceId,
        thread: threadId,
      });
      setLayout({ open: true, tab: "preview" });
    } catch (error) {
      toast.error("Couldn't start the render", (error as Error).message);
    }
  }, [projectId, workspaceId, threadId, setLayout]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        (event.ctrlKey || event.metaKey) &&
        event.key === "Enter" &&
        !event.shiftKey
      ) {
        const target = event.target as HTMLElement;
        if (target.closest("[data-composer]")) return;
        event.preventDefault();
        if (renderingHere || makingVideo)
          toast("Studio is already making this video");
        else if (problem) toast.error("Can't render yet", problem);
        else void render();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [render, renderingHere, makingVideo, problem]);

  if (!project) return null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex h-[52px] shrink-0 items-center gap-3 border-b px-4">
        <div className="flex min-w-0 flex-1 items-center gap-1.5 text-sm">
          <span
            className={cn(
              "truncate font-medium",
              thread && "text-muted-foreground",
            )}
          >
            {project.name}
          </span>
          {thread && (
            <>
              <span className="text-muted-foreground/60">/</span>
              <span className="truncate font-medium">{thread.title}</span>
            </>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <QueueButton />
          {(renderingHere && localRender) || makingVideo ? (
            <Tooltip content="Stop video">
              <Button
                size="sm"
                variant="outline"
                aria-label={`Cancel render. ${localRender?.stage ?? video?.stage ?? "Rendering…"}`}
                onClick={() =>
                  void (
                    makingVideo && video
                      ? api.cancelVideo(video.id)
                      : api.cancelRender(localRender!.id)
                  ).catch((error: Error) =>
                    toast.error("Couldn't cancel", error.message),
                  )
                }
              >
                <Spinner className="size-3.5" />
                <span className="max-w-28 truncate tabular-nums sm:max-w-48">
                  {localRender?.stage ?? video?.stage ?? "Rendering…"}
                </span>
                <SquareIcon className="size-3 fill-current" />
              </Button>
            </Tooltip>
          ) : (
            // A disabled button swallows pointer events, so the tooltip lives on
            // a wrapper and can still explain why Render is unavailable.
            <Tooltip
              content={
                running ? (
                  "Add this video to the render queue"
                ) : problem ? (
                  problem
                ) : (
                  <span className="flex items-center gap-1.5">
                    Render video <Kbd>Ctrl</Kbd>
                    <Kbd>Enter</Kbd>
                  </span>
                )
              }
            >
              <span className="inline-flex">
                <Button
                  size="sm"
                  onClick={() => void render()}
                  disabled={!!problem}
                >
                  <PlayIcon className="fill-current" />{" "}
                  {running ? "Add to queue" : "Render"}
                </Button>
              </span>
            </Tooltip>
          )}
          <Tooltip content={layout.open ? "Hide panel" : "Show panel"}>
            <Button
              size="icon-sm"
              variant="ghost-muted"
              aria-label={layout.open ? "Hide panel" : "Show panel"}
              aria-pressed={layout.open}
              onClick={() => setLayout({ open: !layout.open })}
            >
              {layout.open ? <PanelRightCloseIcon /> : <PanelRightOpenIcon />}
            </Button>
          </Tooltip>
        </div>
      </header>
      <div
        ref={workspace}
        className="studio-workspace relative flex min-h-0 flex-1 overflow-hidden"
        data-panel-open={layout.open}
      >
        <section
          aria-label="Chat"
          className={cn(
            "studio-chat flex min-w-0 flex-1 flex-col",
            layout.open && "studio-chat-with-panel",
          )}
        >
          <ChatPane
            projectId={projectId}
            threadId={threadId}
            onShowPanel={(tab) => setLayout({ open: true, tab })}
          />
        </section>
        <div
          className="studio-panel-shell relative shrink-0"
          data-open={layout.open}
          aria-hidden={!layout.open}
          inert={!layout.open}
          style={
            {
              width: layout.open ? panelWidth : 0,
              "--panel-width": `${panelWidth}px`,
            } as React.CSSProperties
          }
        >
          <div
            className="studio-panel-inner absolute inset-y-0 start-0 flex"
            style={{ width: panelWidth }}
          >
            <ResizeHandle
              width={layout.width}
              onResize={(width) => setLayout({ width })}
            />
            <section
              aria-label="Project panel"
              className="studio-project-panel flex min-w-0 flex-1 flex-col border-l bg-background"
            >
              <RightPanel
                key={workspaceId ?? "project"}
                projectId={projectId}
                threadId={threadId}
                tab={layout.tab}
                onTabChange={(tab) => setLayout({ tab })}
                onRender={() => void render()}
              />
            </section>
          </div>
        </div>
      </div>
    </div>
  );
}

function ResizeHandle({
  width,
  onResize,
}: {
  width: number;
  onResize: (width: number) => void;
}) {
  const [dragging, setDragging] = useState(false);
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Resize panel"
      tabIndex={0}
      aria-valuenow={Math.round(width)}
      aria-valuemin={360}
      aria-valuemax={1400}
      // The visible line stays 4px; the pseudo-element widens the grab area.
      className={cn(
        "studio-resize relative z-10 -me-1 w-1 shrink-0 cursor-col-resize outline-none transition-colors after:absolute after:inset-y-0 after:-inset-x-1.5 after:content-[''] hover:bg-primary/40 focus-visible:bg-primary/60",
        dragging && "bg-primary/60",
      )}
      onKeyDown={(event) => {
        if (event.key === "ArrowLeft") onResize(Math.min(1400, width + 24));
        if (event.key === "ArrowRight") onResize(Math.max(360, width - 24));
      }}
      onPointerDown={(event) => {
        event.preventDefault();
        const start = event.clientX;
        const initial = width;
        const inner = event.currentTarget.parentElement!;
        setDragging(true);
        let latest = initial;
        const move = (next: PointerEvent) => {
          latest = Math.max(
            360,
            Math.min(1400, initial + start - next.clientX),
          );
          // Resize directly while dragging; save the preference once on release.
          const shell = inner.parentElement!;
          const available = shell.parentElement!.clientWidth;
          const width = Math.min(latest, available - 380);
          shell.style.transition = "none";
          shell.style.width = `${width}px`;
          inner.style.width = `${width}px`;
        };
        const up = () => {
          setDragging(false);
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", up);
          document
            .querySelector<HTMLElement>(".studio-panel-shell")
            ?.style.removeProperty("transition");
          onResize(latest);
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", up);
      }}
    />
  );
}
