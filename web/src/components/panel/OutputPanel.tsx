import { FileTextIcon } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Render } from "~/lib/api";
import { latestRender } from "~/lib/render";
import { useStudio } from "~/state/studio";
import { useProjectDoc } from "~/state/project";
import { Spinner } from "../ui/input";
import {
  RepairVideoButton,
  useVideoWorkflow,
  videoIsActive,
} from "../VideoWorkflow";

export function RenderLog({ render }: { render: Render }) {
  const [log, setLog] = useState("");
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setLog("");
    setError("");
    const load = () =>
      void api
        .render(render.id)
        .then((full) => {
          if (active) setLog(full.log || "No log output yet.");
        })
        .catch((error: Error) => {
          if (active) setError(error.message);
        });
    load();
    const timer =
      render.status === "queued" || render.status === "rendering"
        ? window.setInterval(load, 1000)
        : undefined;
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [render.id, render.status]);
  return (
    <div className="min-h-0 flex-1 overflow-auto bg-code p-3">
      {error ? (
        <p role="alert" className="text-sm text-destructive-foreground">
          {error}
        </p>
      ) : !log ? (
        <div className="grid place-items-center py-10">
          <Spinner />
        </div>
      ) : (
        <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">
          {log}
        </pre>
      )}
    </div>
  );
}

export function OutputPanel({ projectId }: { projectId: string }) {
  const video = useVideoWorkflow(projectId);
  const workspace = useProjectDoc((state) => state.workspace);
  const render = useStudio((state) =>
    latestRender(
      state.renders.filter(
        (render) => (render.workspace ?? null) === workspace,
      ),
      projectId,
    ),
  );
  if (!render)
    return (
      <div className="py-14 text-center text-sm text-muted-foreground">
        <FileTextIcon className="mx-auto mb-3 size-7" />
        Render output appears here.
      </div>
    );
  return (
    <>
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-3 text-xs">
        <span className="truncate">{render.name}</span>
        <span className="ms-auto text-muted-foreground capitalize">
          {render.status}
        </span>
      </div>
      {render.status === "failed" && !videoIsActive(video) && (
        <div className="flex items-center justify-between gap-2 border-b px-3 py-2 text-xs">
          <span>Send the error log for automatic repair.</span>
          <RepairVideoButton projectId={projectId} renderId={render.id} />
        </div>
      )}
      <RenderLog render={render} />
    </>
  );
}
