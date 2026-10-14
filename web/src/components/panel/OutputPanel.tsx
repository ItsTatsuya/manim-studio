import { useEffect, useState } from "react";
import { api, type Render } from "~/lib/api";
import { latestRender } from "~/lib/render";
import { useStudio } from "~/state/studio";
import { Spinner } from "../ui/input";

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
        <Spinner />
      ) : (
        <pre className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">
          {log}
        </pre>
      )}
    </div>
  );
}

export function OutputPanel({ projectId }: { projectId: string }) {
  const render = useStudio((state) => latestRender(state.renders, projectId));
  if (!render)
    return (
      <div className="p-6 text-center text-sm text-muted-foreground">
        Render output appears here.
      </div>
    );
  return (
    <>
      <div className="flex h-9 shrink-0 items-center gap-2 border-b px-3 text-xs">
        <span className="truncate">{render.name}</span>
        <span className="ms-auto text-muted-foreground">{render.status}</span>
      </div>
      <RenderLog render={render} />
    </>
  );
}
