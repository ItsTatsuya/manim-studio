import { Link } from "@tanstack/react-router";
import { ClapperboardIcon, MessageSquareIcon } from "lucide-react";
import { useEffect, useState } from "react";

import { relativeTime } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import type { PanelTab } from "../panel/RightPanel";
import { ProviderIcon } from "../icons";
import { Spinner } from "../ui/input";
import { Composer } from "./Composer";
import { Timeline } from "./Timeline";

export function ChatPane({
  projectId,
  threadId,
  onShowPanel,
}: {
  projectId: string;
  threadId?: string;
  onShowPanel: (tab: PanelTab) => void;
}) {
  const detail = useStudio((state) =>
    threadId ? state.details[threadId] : undefined,
  );
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    setMissing(false);
    if (threadId)
      void useStudio
        .getState()
        .loadThread(projectId, threadId)
        .catch(() => setMissing(true));
  }, [projectId, threadId]);

  if (!threadId) return <DraftChat projectId={projectId} />;
  if (missing)
    return (
      <div className="grid flex-1 place-items-center text-muted-foreground text-sm">
        This chat no longer exists.
      </div>
    );
  if (!detail || detail.project !== projectId)
    return (
      <div className="grid flex-1 place-items-center text-muted-foreground">
        <Spinner />
      </div>
    );
  return (
    <>
      <Timeline thread={detail} onShowPanel={onShowPanel} />
      <div className="shrink-0 px-4 pb-4">
        <div className="mx-auto w-full max-w-[46rem]">
          <Composer
            key={`${projectId}:${detail.id}`}
            projectId={projectId}
            thread={detail}
          />
        </div>
      </div>
    </>
  );
}

function DraftChat({ projectId }: { projectId: string }) {
  const project = useStudio((state) =>
    state.projects.find((item) => item.id === projectId),
  );
  const threads = useStudio((state) => state.threads[projectId]);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[46rem] flex-1 flex-col justify-center px-4 py-10">
        <div
          aria-hidden
          className="mx-auto mb-4 grid size-12 place-items-center rounded-2xl bg-primary/10 text-primary ring-1 ring-primary/20"
        >
          <ClapperboardIcon className="size-6" />
        </div>
        <h1 className="text-balance text-center font-semibold text-2xl tracking-tight">
          What should we animate?
        </h1>
        <p className="mx-auto mt-2 mb-6 max-w-md text-pretty text-center text-muted-foreground text-sm">
          Describe the video for{" "}
          <span className="font-medium text-foreground">{project?.name}</span>.
          The agent writes the Manim scene, narration and assets, checks them,
          and Studio renders the result.
        </p>
        <Composer key={projectId} projectId={projectId} showQuickActions />
        {!!threads?.length && (
          <div className="mt-8">
            <div className="mb-1.5 px-1 font-medium text-2xs text-muted-foreground uppercase tracking-wider">
              Recent chats
            </div>
            <div className="divide-y rounded-xl border bg-card/50">
              {threads.slice(0, 5).map((thread) => (
                <Link
                  key={thread.id}
                  to="/projects/$projectId"
                  params={{ projectId }}
                  search={{ thread: thread.id }}
                  className="flex items-center gap-2.5 px-3 py-2 text-sm first:rounded-t-xl last:rounded-b-xl hover:bg-accent"
                >
                  {thread.status === "running" ? (
                    <Spinner className="size-3.5 text-primary" />
                  ) : (
                    <MessageSquareIcon className="size-3.5 text-muted-foreground" />
                  )}
                  <span className="min-w-0 flex-1 truncate">
                    {thread.title}
                  </span>
                  <ProviderIcon
                    provider={thread.provider}
                    className="size-3.5 opacity-70"
                  />
                  <span className="min-w-8 shrink-0 whitespace-nowrap text-right text-muted-foreground text-xs">
                    {relativeTime(thread.updated)}
                  </span>
                </Link>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
