import { useNavigate } from "@tanstack/react-router";
import {
  CheckCircle2Icon,
  CopyIcon,
  EyeIcon,
  NotebookPenIcon,
  PencilIcon,
  SendIcon,
} from "lucide-react";
import { lazy, Suspense, useCallback, useEffect, useState } from "react";

import { api, type Storyboard, type Thread, type Turn } from "~/lib/api";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { useProjectBusy } from "~/state/useProjectBusy";
import { defaultChoice } from "../chat/ModelPicker";
import { Button } from "../ui/button";
import { Badge, Spinner, Textarea } from "../ui/input";
import { toast } from "../ui/toast";
import { Tooltip } from "../ui/tooltip";
import { useVideoWorkflow, videoIsActive } from "../VideoWorkflow";

const Markdown = lazy(() => import("../chat/Markdown"));

/** Review, edit and approve the lesson storyboard written in Plan mode. */
export function StoryboardPanel({
  projectId,
  threadId,
}: {
  projectId: string;
  threadId?: string;
}) {
  const navigate = useNavigate();
  const workspace = useProjectDoc((state) =>
    state.id === projectId ? state.workspace : null,
  );
  const version = useStudio((state) => state.projectVersions[projectId] ?? 0);
  const threads = useStudio((state) => state.threads[projectId]);
  const busy = useProjectBusy(projectId);
  const video = useVideoWorkflow(projectId);
  const [board, setBoard] = useState<Storyboard | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [feedback, setFeedback] = useState("");
  const [working, setWorking] = useState(false);
  const editing = draft !== null;
  const dirty = editing && draft !== board?.text;
  const locked = busy || working || videoIsActive(video);

  const load = useCallback(async () => {
    try {
      setBoard(await api.storyboard(projectId, workspace));
    } catch (error) {
      toast.error("Couldn't load the storyboard", (error as Error).message);
    }
  }, [projectId, workspace]);

  // Reload after the agent (or another window) changes project files.
  useEffect(() => {
    void load();
  }, [load, version]);

  const save = async () => {
    if (draft === null) return board;
    if (!dirty) {
      setDraft(null);
      return board;
    }
    const saved = await api.saveStoryboard(projectId, draft, workspace);
    setBoard(saved);
    setDraft(null);
    return saved;
  };

  /** The chat for this workspace: the open one, the latest, or a new one. */
  const chat = async (): Promise<Thread | { id: string }> => {
    const open = threads?.find(
      (thread) =>
        thread.id === threadId && (thread.workspace ?? null) === workspace,
    );
    if (open) return open;
    const latest = threads?.find(
      (thread) => (thread.workspace ?? null) === workspace,
    );
    if (latest) return latest;
    const created = await api.createThread(projectId, {
      ...defaultChoice(),
      workspace,
      title: "Lesson plan",
    });
    useStudio.setState((state) => ({
      details: { ...state.details, [created.id]: created },
    }));
    useStudio.getState().upsertThreadSummary(created);
    return created;
  };

  const run = async (
    action: (target: { id: string }) => Promise<unknown>,
    failure: string,
  ) => {
    setWorking(true);
    try {
      if (!(await useProjectDoc.getState().flush()))
        throw new Error("Save your project changes first.");
      await save();
      const target = await chat();
      await action(target);
      if (target.id !== threadId)
        void navigate({
          to: "/projects/$projectId",
          params: { projectId },
          search: { thread: target.id },
        });
    } catch (error) {
      toast.error(failure, (error as Error).message);
    } finally {
      setWorking(false);
    }
  };

  const started = (thread: string, turn: Turn) =>
    useStudio.getState().handleEvent({
      type: "thread.turn",
      project: projectId,
      thread,
      turn,
    });

  const revise = () =>
    run(async (target) => {
      const turn = await api.sendTurn(projectId, target.id, {
        text: feedback.trim(),
        mode: "plan",
      });
      started(target.id, turn);
      setFeedback("");
    }, "Couldn't send the feedback");

  const approve = () =>
    run(async (target) => {
      const result = await api.approveStoryboard(
        projectId,
        target.id,
        workspace,
      );
      setBoard(result.storyboard);
      if (result.turn) started(target.id, result.turn);
      toast.success("Storyboard approved", "The agent is building the video.");
    }, "Couldn't approve the storyboard");

  const status = board?.status ?? "none";
  const hasText = !!(editing ? draft : board?.text)?.trim();
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b px-2">
        <span className="px-1 font-mono text-muted-foreground text-xs">
          storyboard.md
        </span>
        {status !== "none" && (
          <Badge
            tone={status === "approved" && !dirty ? "success" : "neutral"}
            data-testid="storyboard-status"
          >
            {status === "approved" && !dirty ? "Approved" : "Draft"}
          </Badge>
        )}
        <div className="ms-auto flex items-center">
          {hasText && (
            <Tooltip content="Copy storyboard">
              <Button
                size="icon-xs"
                variant="ghost-muted"
                aria-label="Copy storyboard"
                onClick={() =>
                  void navigator.clipboard
                    .writeText((editing ? draft : board?.text) ?? "")
                    .then(() => toast.success("Storyboard copied"))
                }
              >
                <CopyIcon />
              </Button>
            </Tooltip>
          )}
          {editing ? (
            <>
              <Button
                size="xs"
                variant="ghost-muted"
                disabled={working}
                onClick={() => setDraft(null)}
              >
                <EyeIcon /> {dirty ? "Discard" : "Preview"}
              </Button>
              <Button
                size="xs"
                variant="outline"
                disabled={locked || !dirty}
                onClick={() =>
                  void save().catch((error: Error) =>
                    toast.error("Couldn't save the storyboard", error.message),
                  )
                }
              >
                Save
              </Button>
            </>
          ) : (
            <Button
              size="xs"
              variant="ghost-muted"
              disabled={locked || !board}
              onClick={() => setDraft(board?.text ?? "")}
            >
              <PencilIcon /> Edit
            </Button>
          )}
        </div>
      </div>
      {editing ? (
        <Textarea
          aria-label="Storyboard markdown"
          className="m-3 min-h-0 flex-1 resize-none font-mono text-xs"
          value={draft}
          readOnly={locked}
          onChange={(event) => setDraft(event.target.value)}
        />
      ) : board?.text.trim() ? (
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <article className="chat-markdown storyboard-markdown text-sm">
            <Suspense
              fallback={<p className="whitespace-pre-wrap">{board.text}</p>}
            >
              <Markdown>{board.text}</Markdown>
            </Suspense>
          </article>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 place-items-center p-6 text-center">
          {board ? (
            <div className="max-w-xs space-y-2">
              <NotebookPenIcon className="mx-auto size-6 text-muted-foreground" />
              <p className="font-medium text-sm">No storyboard yet</p>
              <p className="text-muted-foreground text-xs">
                Switch the chat to <strong>Plan</strong> and describe the
                lesson, or attach a PDF. The agent writes a teaching script with
                scenes for you to review here before any video is made.
              </p>
            </div>
          ) : (
            <Spinner />
          )}
        </div>
      )}
      {hasText && (
        <div className="flex shrink-0 flex-col gap-2 border-t p-3">
          <Textarea
            aria-label="Storyboard feedback"
            className="min-h-14"
            rows={2}
            value={feedback}
            disabled={locked}
            placeholder="What should change? e.g. “Add a worked example for parallel lines”"
            onChange={(event) => setFeedback(event.target.value)}
          />
          <div className="flex flex-wrap items-center justify-end gap-2">
            {(busy || working) && (
              <span className="me-auto flex items-center gap-1.5 text-muted-foreground text-xs">
                <Spinner className="size-3" /> The agent is working…
              </span>
            )}
            <Button
              size="sm"
              variant="outline"
              disabled={locked || !feedback.trim()}
              onClick={() => void revise()}
            >
              <SendIcon /> Revise with agent
            </Button>
            <Button size="sm" disabled={locked} onClick={() => void approve()}>
              <CheckCircle2Icon /> Approve &amp; generate video
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
