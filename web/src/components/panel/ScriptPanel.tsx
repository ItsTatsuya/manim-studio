import { CopyIcon, SearchIcon, SparklesIcon } from "lucide-react";
import { lazy, Suspense, useEffect, useMemo, useRef, useState } from "react";

import { api } from "~/lib/api";
import { latestRender } from "~/lib/render";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { useProjectBusy } from "~/state/useProjectBusy";
import { Button } from "../ui/button";
import { toast } from "../ui/toast";
import { Tooltip } from "../ui/tooltip";
import type { CodeEditorHandle, Problem } from "./CodeEditor";
import {
  RepairVideoButton,
  useVideoWorkflow,
  videoIsActive,
} from "../VideoWorkflow";

// CodeMirror is the largest dependency; load it when a script is first shown.
const CodeEditor = lazy(() =>
  import("./CodeEditor").then((module) => ({ default: module.CodeEditor })),
);

export function ScriptPanel({ projectId }: { projectId: string }) {
  const source = useProjectDoc((state) => state.source);
  const revision = useProjectDoc((state) => state.revision);
  const analysis = useProjectDoc((state) => state.analysis);
  const workspace = useProjectDoc((state) => state.workspace);
  const loaded = useProjectDoc((state) => state.project?.id === projectId);
  const last = useStudio((state) =>
    latestRender(
      state.renders.filter(
        (render) => (render.workspace ?? null) === workspace,
      ),
      projectId,
    ),
  );
  const editor = useRef<CodeEditorHandle>(null);
  const [cursor, setCursor] = useState({ line: 1, column: 1 });
  const busy = useProjectBusy(projectId);
  const video = useVideoWorkflow(projectId);
  const error =
    analysis?.error ||
    (last?.status === "failed" && (last.problem?.message || last.error));

  const problems = useMemo<Problem[]>(() => {
    const list: Problem[] = [];
    if (analysis?.error && analysis.line)
      list.push({ line: analysis.line, message: analysis.error });
    if (last?.status === "failed" && last.problem)
      list.push({ line: last.problem.line, message: last.problem.message });
    return list;
  }, [analysis, last]);

  // Opening the tab after a failed render goes straight to the failing line.
  useEffect(() => {
    if (loaded && last?.status === "failed" && last.problem)
      editor.current?.jump(last.problem.line);
  }, [loaded]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!loaded) return null;

  const format = async () => {
    try {
      const result = await api.format(source);
      if (
        result.source !== source &&
        useProjectDoc.getState().source === source &&
        !useStudio.getState().installing &&
        !(useStudio.getState().threads[projectId] ?? []).some(
          (thread) =>
            thread.status === "running" &&
            (workspace === null ||
              !thread.workspace ||
              thread.workspace === workspace),
        )
      )
        useProjectDoc.getState().replaceSource(result.source);
    } catch (error) {
      toast.error("Couldn't format", (error as Error).message);
    }
  };

  const scenes = analysis?.scenes ?? [];
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b px-2">
        <span className="px-1 font-mono text-muted-foreground text-xs">
          script.py
        </span>
        <div className="ms-auto flex items-center">
          <Tooltip content="Find (Ctrl+F)">
            <Button
              size="icon-xs"
              variant="ghost-muted"
              aria-label="Find"
              onClick={() => editor.current?.find()}
            >
              <SearchIcon />
            </Button>
          </Tooltip>
          <Tooltip content="Copy script">
            <Button
              size="icon-xs"
              variant="ghost-muted"
              aria-label="Copy script"
              onClick={() =>
                void navigator.clipboard
                  .writeText(source)
                  .then(() => toast.success("Script copied"))
              }
            >
              <CopyIcon />
            </Button>
          </Tooltip>
          <Button
            size="xs"
            variant="ghost-muted"
            disabled={busy}
            onClick={() => void format()}
          >
            <SparklesIcon /> Format
          </Button>
        </div>
      </div>
      <Suspense fallback={<div className="min-h-0 flex-1" />}>
        <CodeEditor
          ref={editor}
          value={source}
          revision={revision}
          readOnly={busy}
          problems={problems}
          onChange={(value) => useProjectDoc.getState().setSource(value)}
          onCursor={(line, column) => setCursor({ line, column })}
        />
      </Suspense>
      {error && !!source.trim() && !videoIsActive(video) && (
        <div className="flex flex-wrap items-center gap-2 border-t px-3 py-2 text-xs">
          <span className="min-w-0 flex-1 break-words text-destructive-foreground">
            {error}
          </span>
          <RepairVideoButton
            projectId={projectId}
            renderId={last?.status === "failed" ? last.id : undefined}
          />
        </div>
      )}
      <div
        className="flex h-7 shrink-0 items-center gap-3 border-t px-3 text-2xs text-muted-foreground"
        id="script-status"
      >
        {analysis?.error ? (
          <button
            type="button"
            className="truncate text-destructive-foreground hover:underline"
            onClick={() => analysis.line && editor.current?.jump(analysis.line)}
          >
            {analysis.error}
          </button>
        ) : (
          <span>
            {scenes.length === 1 ? "1 scene" : `${scenes.length} scenes`}
            {scenes.length ? ` · ${scenes.join(", ")}` : ""}
          </span>
        )}
        <span className="ms-auto shrink-0 tabular-nums">
          Ln {cursor.line}, Col {cursor.column}
        </span>
      </div>
    </div>
  );
}
