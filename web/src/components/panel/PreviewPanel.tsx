import {
  CircleAlertIcon,
  DownloadIcon,
  FolderOpenIcon,
  PlayIcon,
  RepeatIcon,
  SquareIcon,
} from "lucide-react";
import { useState } from "react";

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
import { Spinner } from "../ui/input";
import { Select } from "../ui/select";
import { toast } from "../ui/toast";
import type { PanelTab } from "./RightPanel";
import { OutputSettings } from "./OutputSettings";

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
  const health = useStudio((state) => state.health);
  const analysis = useProjectDoc((state) => state.analysis);
  const scene = useProjectDoc((state) => state.scene);
  const running = activeRender(renders);
  const mine = running?.project === projectId ? running : null;
  const last = latestRender(renders, projectId);
  const shown = renders.find(
    (render) => render.project === projectId && render.status === "completed",
  );
  const preset = currentPreset(prefs);
  const [loop, setLoop] = useState(false);
  const [custom, setCustom] = useState(false);
  const problem = renderProblem(analysis, health);

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
              autoPlay={
                !!last &&
                last.id === shown.id &&
                Date.now() - new Date(shown.created).getTime() < 10 * 60_000
              }
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

        {last?.status === "failed" && !mine && (
          <div
            role="alert"
            className="mt-3 rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-[13px]"
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

      <div className="mt-auto border-t p-3">
        <div className="grid grid-cols-[1fr_1fr_auto] items-end gap-2">
          <label className="flex min-w-0 flex-col gap-1">
            <span className="text-[11px] text-muted-foreground">Scene</span>
            <Select
              aria-label="Scene"
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
          </label>
          <label className="flex min-w-0 flex-col gap-1">
            <span className="text-[11px] text-muted-foreground">Output</span>
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
          </label>
          <Button
            size="sm"
            onClick={onRender}
            disabled={!!running || !!problem}
          >
            <PlayIcon className="fill-current" /> Render
          </Button>
        </div>
        {problem && (
          <p className="mt-2 text-destructive-foreground text-xs">{problem}</p>
        )}
        {running && !mine && (
          <p className="mt-2 text-muted-foreground text-xs">
            Another project is rendering.
          </p>
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
        {custom && (
          <OutputSettings
            open={custom}
            onOpenChange={setCustom}
            value={preset}
          />
        )}
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
