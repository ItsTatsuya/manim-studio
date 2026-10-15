import { useNavigate, useRouterState } from "@tanstack/react-router";
import { SquareIcon, WrenchIcon } from "lucide-react";
import { useState } from "react";

import { api, type VideoWorkflow } from "~/lib/api";
import { renderProject } from "~/lib/render";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { Button } from "./ui/button";
import { Spinner } from "./ui/input";
import { toast } from "./ui/toast";

export function videoIsActive(video?: VideoWorkflow) {
  return (
    !!video && ["checking", "repairing", "rendering"].includes(video.status)
  );
}

export function useVideoWorkflow(
  projectId: string,
  workspaceOverride?: string | null,
) {
  const openWorkspace = useProjectDoc((state) =>
    state.id === projectId ? state.workspace : null,
  );
  const workspace =
    workspaceOverride === undefined ? openWorkspace : workspaceOverride;
  return useStudio((state) =>
    state.videos.find(
      (video) => video.project === projectId && video.workspace === workspace,
    ),
  );
}

export function RepairVideoButton({
  projectId,
  renderId,
  workspace,
}: {
  projectId: string;
  renderId?: string;
  workspace?: string | null;
}) {
  const [sending, setSending] = useState(false);
  const video = useVideoWorkflow(projectId, workspace);
  const thread = useRouterState({
    select: (state) =>
      state.location.pathname === `/projects/${projectId}`
        ? (state.location.search as { thread?: string }).thread
        : undefined,
  });
  const navigate = useNavigate();
  const repair = async () => {
    setSending(true);
    try {
      const result = await renderProject(projectId, {
        repair: true,
        render: renderId,
        thread,
        workspace,
      });
      if (result.thread && result.thread !== thread) {
        await useStudio.getState().loadThreads(projectId);
        void navigate({
          to: "/projects/$projectId",
          params: { projectId },
          search: { thread: result.thread },
        });
      }
    } catch (error) {
      toast.error("Couldn't send the error", (error as Error).message);
    } finally {
      setSending(false);
    }
  };
  return (
    <Button
      size="xs"
      variant="outline"
      disabled={sending || videoIsActive(video)}
      onClick={() => void repair()}
    >
      {sending ? <Spinner className="size-3.5" /> : <WrenchIcon />} Send to LLM
    </Button>
  );
}

export function VideoWorkflowNotice({ projectId }: { projectId: string }) {
  const video = useVideoWorkflow(projectId);
  if (
    !video ||
    (!videoIsActive(video) && video.status !== "failed" && !video.warning)
  )
    return null;
  const active = videoIsActive(video);
  return (
    <div
      className="flex flex-wrap items-start gap-2 rounded-lg border bg-muted/30 px-3 py-2 text-xs"
      role="status"
    >
      {active && <Spinner className="mt-0.5 size-3.5 shrink-0" />}
      <div className="min-w-0 flex-1">
        <div className="font-medium">{video.stage}</div>
        {video.status === "repairing" && (
          <p className="mt-1 text-muted-foreground">
            The agent is fixing the error. Studio will retry the render
            automatically.
          </p>
        )}
        {video.status === "failed" && (
          <p className="mt-1 break-words text-muted-foreground">
            {video.error}
          </p>
        )}
        {video.warning && !active && (
          <p className="mt-1 text-muted-foreground">{video.warning}</p>
        )}
      </div>
      {active ? (
        <Button
          size="icon-xs"
          variant="ghost-muted"
          aria-label="Stop video"
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
      ) : (
        video.status === "failed" && (
          <RepairVideoButton
            projectId={projectId}
            renderId={video.render ?? undefined}
          />
        )
      )}
    </div>
  );
}
