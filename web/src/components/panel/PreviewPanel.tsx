import {
  CircleAlertIcon,
  DownloadIcon,
  FolderOpenIcon,
  PlayIcon,
  RepeatIcon,
  SquareIcon,
} from "lucide-react";
import { useEffect, useState } from "react";

import { api } from "~/lib/api";
import {
  OUTPUT_PRESETS,
  activeRender,
  choosePreset,
  currentPreset,
  latestRender,
  renderProblem,
} from "~/lib/render";
import { formatBytes, formatDuration } from "~/lib/utils";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { useElapsed } from "../chat/useElapsed";
import { Button } from "../ui/button";
import { Field } from "../ui/field";
import { Spinner } from "../ui/input";
import { Select } from "../ui/select";
import { toast } from "../ui/toast";
import type { PanelTab } from "./RightPanel";
import { OutputSettings } from "./OutputSettings";
import {
  RepairVideoButton,
  useVideoWorkflow,
  videoIsActive,
  VideoWorkflowNotice,
} from "../VideoWorkflow";

// Renders that already started playing on their own. The panel remounts on every
// tab switch, so without this a fresh render would restart each time you return.
const autoplayed = new Set<string>();

export function PreviewPanel({
  projectId,
  onRender,
  onShowTab,
}: {
  projectId: string;
  onRender: () => void;
  onShowTab: (tab: PanelTab) => void;
}) {
  const renders = useStudio((state) => state.renders);
  const prefs = useStudio((state) => state.prefs);
  const queue = useStudio((state) => state.queue);
  const health = useStudio((state) => state.health);
  const analysis = useProjectDoc((state) => state.analysis);
  const scene = useProjectDoc((state) => state.scene);
  const workspace = useProjectDoc((state) => state.workspace);
  const scopedRenders = renders.filter(
    (render) =>
      render.project === projectId && (render.workspace ?? null) === workspace,
  );
  const running = activeRender(renders);
  const mine = activeRender(scopedRenders);
  const last = latestRender(scopedRenders, projectId);
  const shown = scopedRenders.find(
    (render) => render.project === projectId && render.status === "completed",
  );
  const preset = currentPreset(prefs);
  const [loop, setLoop] = useState(false);
  const [custom, setCustom] = useState(false);
  const problem = renderProblem(analysis, health);
  const video = useVideoWorkflow(projectId);
  const activeVideo = videoIsActive(video);
  const source = useProjectDoc((state) => state.source);
  const multipleScenes = (analysis?.scenes.length ?? 0) > 1;
  const autoPlay =
    !!shown &&
    !!last &&
    last.id === shown.id &&
    !autoplayed.has(shown.id) &&
    Date.now() - new Date(shown.created).getTime() < 10 * 60_000;
  useEffect(() => {
    if (autoPlay && shown) autoplayed.add(shown.id);
  }, [autoPlay, shown]);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="p-3">
        <div className="relative aspect-video overflow-hidden rounded-xl border bg-black">
          {shown?.video ? (
            <video
              key={shown.id}
              src={`${shown.video}?v=${shown.id}`}
              controls
              loop={loop}
              autoPlay={autoPlay}
              className="size-full object-contain"
              aria-label={`Rendered video: ${shown.name}`}
            />
          ) : (
            <div className="grid size-full place-items-center text-center text-sm text-white/60">
              <div>
                <PlayIcon className="mx-auto mb-2 size-8 opacity-50" />
                Render to see your animation
              </div>
            </div>
          )}
          {mine && <RenderProgress render={mine} />}
        </div>

        <div className="mt-3">
          <VideoWorkflowNotice projectId={projectId} />
        </div>
        {last?.status === "failed" &&
          !mine &&
          !problem &&
          !activeVideo &&
          video?.status !== "failed" && (
            <div
              role="alert"
              className="mt-3 rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-ui"
            >
              <div className="flex items-start gap-2 text-destructive-foreground">
                <CircleAlertIcon className="mt-0.5 size-4 shrink-0" />
                <div className="min-w-0">
                  <div className="font-medium">The render needs attention</div>
                  <div className="mt-0.5 break-words text-foreground/80">
                    {last.problem
                      ? `Line ${last.problem.line}: ${last.problem.message}`
                      : last.error}
                  </div>
                </div>
              </div>
              <div className="mt-2 flex gap-1.5 ps-6">
                <RepairVideoButton projectId={projectId} renderId={last.id} />
                {last.problem && (
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => onShowTab("script")}
                  >
                    Go to line {last.problem.line}
                  </Button>
                )}
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() => onShowTab("output")}
                >
                  Show log
                </Button>
              </div>
            </div>
          )}

        {shown && (
          <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1">
            <div className="min-w-0 flex-1">
              <div className="truncate font-medium text-sm">{shown.name}</div>
              <div className="text-muted-foreground text-xs">
                {[
                  shown.resolution,
                  shown.fps ? `${Math.round(shown.fps * 100) / 100} fps` : null,
                  shown.narrated ? "Narrated" : null,
                  shown.elapsed
                    ? `Rendered in ${formatDuration(shown.elapsed)}`
                    : null,
                  shown.size ? formatBytes(shown.size) : null,
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </div>
            </div>
            <div className="flex items-center gap-1">
              <Button
                size="xs"
                variant={loop ? "secondary" : "ghost-muted"}
                aria-pressed={loop}
                onClick={() => setLoop(!loop)}
              >
                <RepeatIcon /> Loop
              </Button>
              <Button
                size="xs"
                variant="outline"
                render={<a href={`${shown.video}?download=true`} download />}
              >
                <DownloadIcon /> Save video
              </Button>
              <Button
                size="xs"
                variant="ghost-muted"
                onClick={() => void api.openOutputs()}
              >
                <FolderOpenIcon /> Folder
              </Button>
            </div>
          </div>
        )}
      </div>

      <div className="sticky bottom-0 mt-auto border-t bg-card/60 p-3 backdrop-blur supports-[backdrop-filter]:bg-card/40">
        <div
          className={`grid ${multipleScenes ? "grid-cols-[1fr_1fr_auto]" : "grid-cols-[1fr_auto]"} items-end gap-3`}
        >
          {multipleScenes && (
            <Field label="Animation">
              <Select
                aria-label="Animation"
                size="sm"
                value={scene}
                disabled={!analysis?.scenes.length}
                placeholder={analysis?.error ? "No scene" : "Detecting…"}
                onChange={(value) => useProjectDoc.getState().setScene(value)}
                options={(analysis?.scenes ?? []).map((name) => ({
                  value: name,
                  label: name,
                }))}
              />
            </Field>
          )}
          <Field label="Quality">
            <Select
              aria-label="Output quality"
              size="sm"
              value={preset.id}
              onChange={(id) =>
                id === "custom"
                  ? setCustom(true)
                  : choosePreset(OUTPUT_PRESETS.find((item) => item.id === id)!)
              }
              options={[
                ...OUTPUT_PRESETS.map((item) => ({
                  value: item.id,
                  label: `${item.label} · ${item.detail}`,
                })),
                {
                  value: "custom",
                  label:
                    preset.id === "custom"
                      ? `Custom · ${preset.detail}`
                      : "Custom…",
                },
              ]}
            />
          </Field>
          <Button
            size="sm"
            className="min-w-28"
            onClick={onRender}
            disabled={!!mine || !!problem || activeVideo}
          >
            <PlayIcon className="fill-current" />{" "}
            {running && !mine ? "Add to queue" : "Render"}
          </Button>
        </div>
        {problem &&
          !!source.trim() &&
          !activeVideo &&
          video?.status !== "failed" && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <p className="min-w-0 flex-1 text-destructive-foreground text-xs">
                {problem}
              </p>
              {analysis && <RepairVideoButton projectId={projectId} />}
            </div>
          )}
        {!source.trim() && !activeVideo && !mine && (
          <p className="mt-2 text-muted-foreground text-xs">
            Ask the agent for a scene, or add one in Script, to enable Render.
          </p>
        )}
        {running && !mine && (
          <p className="mt-2 text-muted-foreground text-xs">
            Another project is rendering.
          </p>
        )}
        {queue?.paused && queue.waiting.length > 0 && (
          <div className="mt-2 flex items-center justify-between gap-2 text-muted-foreground text-xs">
            <span>
              {queue.waiting.length}{" "}
              {queue.waiting.length === 1 ? "video is" : "videos are"} waiting
              in the render queue.
            </span>
            <Button
              size="xs"
              variant="outline"
              onClick={() =>
                void api
                  .startQueue()
                  .catch((error: Error) =>
                    toast.error("Couldn't start the queue", error.message),
                  )
              }
            >
              Start queue
            </Button>
          </div>
        )}
        {preset.id === "custom" && (
          <Button
            size="xs"
            variant="ghost-muted"
            onClick={() => setCustom(true)}
          >
            Edit custom output
          </Button>
        )}
        <OutputSettings open={custom} onOpenChange={setCustom} value={preset} />
      </div>
    </div>
  );
}

function RenderProgress({
  render,
}: {
  render: NonNullable<ReturnType<typeof activeRender>>;
}) {
  const elapsed = useElapsed(render.created);
  const percent = Number(render.stage?.match(/(\d+)%/)?.[1] ?? NaN);
  return (
    <div
      className="absolute inset-0 grid place-items-center bg-black/70 backdrop-blur-[1px]"
      role="status"
    >
      <div className="w-64 text-center text-white">
        <Spinner className="mx-auto mb-3 size-6" />
        <div className="truncate font-medium text-sm">
          {render.stage ?? "Preparing…"}
        </div>
        <div className="mt-2 h-1 overflow-hidden rounded-full bg-white/15">
          <div
            className="h-full rounded-full bg-primary transition-[width] duration-300"
            style={{ width: Number.isFinite(percent) ? `${percent}%` : "35%" }}
          />
        </div>
        <div className="mt-2 text-white/60 text-xs tabular-nums">
          {formatDuration(elapsed)}
        </div>
        {render.can_cancel !== false && (
          <Button
            size="xs"
            variant="secondary"
            className="mt-3 bg-white/10 text-white hover:bg-white/20"
            onClick={() =>
              void api
                .cancelRender(render.id)
                .catch((error: Error) =>
                  toast.error("Couldn't cancel", error.message),
                )
            }
          >
            <SquareIcon className="size-2.5 fill-current" /> Cancel
          </Button>
        )}
      </div>
    </div>
  );
}
