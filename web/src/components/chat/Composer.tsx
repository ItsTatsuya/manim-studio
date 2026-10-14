import { Link, useNavigate } from "@tanstack/react-router";
import {
  ArrowUpIcon,
  FileTextIcon,
  ImageIcon,
  PaperclipIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  SquareIcon,
  WrenchIcon,
  XIcon,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";

import { api, type Thread } from "~/lib/api";
import {
  FIX_RENDER_PROMPT,
  QUICK_ACTIONS,
  fixRenderContext,
} from "~/lib/prompts";
import { latestRender } from "~/lib/render";
import { cn } from "~/lib/utils";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { Button } from "../ui/button";
import { Select } from "../ui/select";
import { Spinner } from "../ui/input";
import { Menu, MenuItem, MenuLabel, MenuPopup, MenuTrigger } from "../ui/menu";
import { toast } from "../ui/toast";
import { Tooltip } from "../ui/tooltip";
import {
  ModelPicker,
  defaultChoice,
  providerReadiness,
  type ModelChoice,
} from "./ModelPicker";

const drafts = new Map<string, string>();

export function Composer({
  projectId,
  thread,
  showQuickActions,
}: {
  projectId: string;
  thread?: Thread;
  showQuickActions?: boolean;
}) {
  const navigate = useNavigate();
  const draftKey = thread?.id ?? `new:${projectId}`;
  const [text, setText] = useState(() => drafts.get(draftKey) ?? "");
  const [context, setContext] = useState<{
    label: string;
    text: string;
  } | null>(null);
  const [attachments, setAttachments] = useState<string[]>([]);
  const [uploading, setUploading] = useState(0);
  const [pickingFiles, setPickingFiles] = useState(false);
  const pendingUploads = useRef(0);
  const draftThread = useRef<Thread | null>(null);
  const [sending, setSending] = useState(false);
  const [choice, setChoice] = useState<ModelChoice>(() =>
    thread
      ? {
          provider: thread.provider,
          model: thread.model,
          effort: thread.effort,
        }
      : defaultChoice(),
  );
  const agents = useStudio((state) => state.agents);
  const nativePicker = useStudio(
    (state) => state.health?.native_file_picker ?? false,
  );
  const renders = useStudio((state) => state.renders);
  const access = agents?.settings.access ?? "full";
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const mediaInput = useRef<HTMLInputElement>(null);
  const running = thread?.status === "running";
  const status = agents?.providers.find(
    (provider) => provider.id === choice.provider,
  );
  const readiness = providerReadiness(status);
  const lastRender = latestRender(renders, projectId);
  const canFix = lastRender?.status === "failed";

  useEffect(() => {
    setText(drafts.get(draftKey) ?? "");
    setContext(null);
    setAttachments([]);
  }, [draftKey]);

  useEffect(() => {
    if (thread)
      setChoice({
        provider: thread.provider,
        model: thread.model,
        effort: thread.effort,
      });
  }, [thread?.id, thread?.provider, thread?.model, thread?.effort]); // eslint-disable-line react-hooks/exhaustive-deps

  // A new chat follows the settings until the user picks something else.
  const [touched, setTouched] = useState(false);
  useEffect(() => {
    if (!thread && !touched && agents) setChoice(defaultChoice());
  }, [agents, thread, touched]);

  useEffect(() => {
    const element = input.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 260)}px`;
  }, [text]);

  const updateText = (value: string) => {
    setText(value);
    drafts.set(draftKey, value);
  };

  const changeChoice = async (next: ModelChoice) => {
    setTouched(true);
    setChoice(next);
    const target = thread ?? draftThread.current;
    if (target)
      try {
        const summary = await api.updateThread(projectId, target.id, {
          provider: next.provider,
          model: next.model,
          effort: next.effort,
        });
        if (draftThread.current?.id === target.id)
          draftThread.current = { ...draftThread.current, ...summary };
        useStudio.getState().upsertThreadSummary(summary);
      } catch (error) {
        toast.error("Couldn't change the model", (error as Error).message);
      }
  };

  const contextText = useMemo(() => {
    const parts = [];
    if (context) parts.push(context.text);
    if (attachments.length)
      parts.push(
        `Reference files the user added to the project (read their contents before using them):\n${attachments.map((path) => JSON.stringify(`assets/${path}`)).join("\n")}`,
      );
    return parts.join("\n\n");
  }, [context, attachments]);

  const send = async (override?: string, overrideContext?: string) => {
    const message = (override ?? text).trim();
    if (!message || sending || running || pendingUploads.current > 0) return;
    if (!readiness.ready) {
      toast.error(
        `${status?.name ?? "The agent"} isn't ready`,
        status?.message || "Open Settings → Providers.",
      );
      return;
    }
    setSending(true);
    try {
      if (!(await useProjectDoc.getState().flush()))
        throw new Error("Save your project changes before sending a message.");
      let target = thread ?? draftThread.current;
      if (!target) {
        target = await api.createThread(projectId, choice);
        draftThread.current = target;
        useStudio.setState((state) => ({
          details: { ...state.details, [target!.id]: target! },
        }));
        useStudio.getState().upsertThreadSummary(target);
      }
      const turn = await api.sendTurn(projectId, target.id, {
        text: message,
        context: overrideContext ?? contextText,
      });
      useStudio.getState().handleEvent({
        type: "thread.turn",
        project: projectId,
        thread: target.id,
        turn,
      });
      drafts.delete(draftKey);
      setText("");
      setContext(null);
      setAttachments([]);
      if (!thread)
        void navigate({
          to: "/projects/$projectId",
          params: { projectId },
          search: { thread: target.id },
        });
    } catch (error) {
      toast.error("Couldn't send the message", (error as Error).message);
    } finally {
      setSending(false);
    }
  };

  const stop = () => {
    if (thread)
      void api
        .interrupt(projectId, thread.id)
        .catch((error: Error) =>
          toast.error("Couldn't stop the agent", error.message),
        );
  };

  const attach = async (files: File[]) => {
    if (!files.length || sending) return;
    pendingUploads.current += files.length;
    setUploading(pendingUploads.current);
    for (const file of files) {
      try {
        const asset = await api.uploadAsset(projectId, file);
        useStudio.getState().handleEvent({
          type: "project.changed",
          project: projectId,
          files: [`assets/${asset.path}`],
        });
        setAttachments((current) => [...current, asset.path]);
      } catch (error) {
        toast.error(`Couldn't add ${file.name}`, (error as Error).message);
      } finally {
        pendingUploads.current -= 1;
        setUploading(pendingUploads.current);
      }
    }
  };

  const chooseFiles = (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = [...(event.target.files ?? [])];
    event.target.value = "";
    void attach(files);
  };

  const openPicker = async (kind: "documents" | "media") => {
    if (sending || pickingFiles) return;
    if (!nativePicker) {
      const element =
        kind === "documents" ? fileInput.current : mediaInput.current;
      if (element?.showPicker) element.showPicker();
      else element?.click();
      return;
    }
    pendingUploads.current += 1;
    setUploading(pendingUploads.current);
    setPickingFiles(true);
    try {
      const result = await api.chooseAssets(projectId, kind);
      for (const error of result.errors)
        toast.error(`Couldn't add ${error.file}`, error.message);
      if (result.assets.length) {
        setAttachments((current) => [
          ...current,
          ...result.assets.map((asset) => asset.path),
        ]);
        useStudio.getState().handleEvent({
          type: "project.changed",
          project: projectId,
          files: result.assets.map((asset) => `assets/${asset.path}`),
        });
      }
    } catch (error) {
      toast.error("Couldn't choose files", (error as Error).message);
    } finally {
      pendingUploads.current -= 1;
      setUploading(pendingUploads.current);
      setPickingFiles(false);
    }
  };

  const fixError = async () => {
    if (!lastRender) return;
    const full = await api.render(lastRender.id).catch(() => lastRender);
    setContext({
      label: "Last render error",
      text: fixRenderContext(full, full.log ?? ""),
    });
    updateText(FIX_RENDER_PROMPT);
    input.current?.focus();
  };

  return (
    <div className="flex flex-col gap-2">
      {showQuickActions && !thread && (
        <div className="mb-1 flex flex-wrap justify-center gap-1.5">
          {QUICK_ACTIONS.map((action) => (
            <Button
              key={action.id}
              size="xs"
              variant="outline"
              className="rounded-full px-2.5"
              onClick={() => {
                if (action.immediate) void send(action.prompt);
                else {
                  updateText(action.prompt);
                  window.requestAnimationFrame(() => {
                    const element = input.current;
                    if (!element) return;
                    element.focus();
                    const at = action.prompt.indexOf(": ") + 2;
                    element.setSelectionRange(
                      at > 1 ? at : action.prompt.length,
                      at > 1 ? at : action.prompt.length,
                    );
                  });
                }
              }}
            >
              {action.label}
            </Button>
          ))}
          {canFix && (
            <Button
              size="xs"
              variant="outline"
              className="rounded-full px-2.5"
              onClick={() => void fixError()}
            >
              <WrenchIcon /> Fix render error
            </Button>
          )}
        </div>
      )}
      {thread && canFix && !running && !context && (
        <div className="flex items-center justify-between gap-2 rounded-lg border border-destructive/25 bg-destructive/5 px-3 py-1.5 text-[13px]">
          <span className="text-destructive-foreground">
            The last render failed.
          </span>
          <Button size="xs" variant="outline" onClick={() => void fixError()}>
            <WrenchIcon /> Ask the agent to fix it
          </Button>
        </div>
      )}
      <div
        data-composer
        className={cn(
          "rounded-[20px] border bg-card shadow-composer transition-colors focus-within:border-ring/50 dark:bg-[color-mix(in_srgb,var(--background)_94%,white)]",
          !readiness.ready && agents && "border-warning/40",
        )}
        onDragOver={(event) => event.preventDefault()}
        onDrop={(event) => {
          event.preventDefault();
          void attach([...event.dataTransfer.files]);
        }}
      >
        {(context || attachments.length > 0) && (
          <div className="flex flex-wrap gap-1.5 px-3 pt-3">
            {context && (
              <Chip label={context.label} onRemove={() => setContext(null)} />
            )}
            {attachments.map((path) => (
              <Chip
                key={path}
                label={path}
                onRemove={() =>
                  setAttachments((current) =>
                    current.filter((item) => item !== path),
                  )
                }
              />
            ))}
          </div>
        )}
        {uploading > 0 && (
          <div
            role="status"
            className="flex items-center gap-2 px-4 pt-3 text-xs text-muted-foreground"
          >
            <Spinner className="size-3" />
            {pickingFiles
              ? "Choose files in the file picker…"
              : `Uploading ${uploading} ${uploading === 1 ? "file" : "files"}…`}
          </div>
        )}
        <textarea
          ref={input}
          aria-label="Message the agent"
          value={text}
          rows={thread ? 1 : 3}
          placeholder={
            thread
              ? "Ask for changes…"
              : "Describe your video, e.g. “Explain how a Fourier series builds a square wave, 45 seconds, with narration”"
          }
          onChange={(event) => updateText(event.target.value)}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault();
              void send();
            }
          }}
          className="block max-h-[260px] min-h-[44px] w-full resize-none bg-transparent px-4 pt-3 pb-1 text-[14px] leading-relaxed outline-none placeholder:text-muted-foreground/70"
        />
        <div className="flex items-center gap-1 px-2 pb-2">
          <ModelPicker
            value={choice}
            onChange={(next) => void changeChoice(next)}
            lockedProvider={thread?.locked ? thread.provider : undefined}
          />
          <Select
            aria-label="Agent access"
            size="xs"
            variant="ghost"
            value={access}
            onChange={(value) =>
              void api
                .saveAgentSettings({ access: value })
                .then(() => useStudio.getState().loadAgents())
            }
            options={[
              {
                value: "full",
                label: (
                  <span className="flex items-center gap-1">
                    <ShieldAlertIcon className="size-3" />
                    Full access
                  </span>
                ),
                description: "Edit files and run checks without sandboxing",
              },
              {
                value: "edits",
                label: (
                  <span className="flex items-center gap-1">
                    <ShieldCheckIcon className="size-3" />
                    Auto edits
                  </span>
                ),
                description:
                  "Codex workspace sandbox or Claude acceptEdits permissions",
              },
            ]}
          />
          <div className="ms-auto flex items-center gap-1">
            <Menu>
              <MenuTrigger
                render={<Button size="icon-sm" variant="ghost-muted" />}
                aria-label="Attach files"
                disabled={sending || pickingFiles}
              >
                <PaperclipIcon />
              </MenuTrigger>
              <MenuPopup side="top" align="end">
                <MenuItem onClick={() => void openPicker("documents")}>
                  <FileTextIcon /> Upload documents
                </MenuItem>
                <MenuItem onClick={() => void openPicker("media")}>
                  <ImageIcon /> Upload images or audio
                </MenuItem>
                <MenuLabel>Or drag files into your message</MenuLabel>
              </MenuPopup>
            </Menu>
            {running ? (
              <Tooltip content="Stop the agent">
                <Button
                  size="icon-sm"
                  variant="secondary"
                  aria-label="Stop"
                  className="rounded-full"
                  onClick={stop}
                >
                  <SquareIcon className="size-3 fill-current" />
                </Button>
              </Tooltip>
            ) : (
              <Button
                size="icon-sm"
                aria-label="Send"
                className="rounded-full"
                disabled={!text.trim() || sending || uploading > 0}
                onClick={() => void send()}
              >
                <ArrowUpIcon />
              </Button>
            )}
          </div>
        </div>
        <input
          ref={fileInput}
          type="file"
          aria-label="Upload reference files"
          multiple
          hidden
          onChange={chooseFiles}
        />
        <input
          ref={mediaInput}
          type="file"
          aria-label="Upload media files"
          multiple
          hidden
          accept="image/*,.svg,audio/*,video/*"
          onChange={chooseFiles}
        />
      </div>
      {agents && !readiness.ready && (
        <p className="px-2 text-center text-warning-foreground text-xs">
          {status?.message || `${status?.name ?? "This agent"} is not ready.`}{" "}
          <Link
            to="/settings/providers"
            className="underline underline-offset-2"
          >
            Set up providers
          </Link>
        </p>
      )}
    </div>
  );
}

function Chip({ label, onRemove }: { label: string; onRemove: () => void }) {
  return (
    <span className="inline-flex h-6 items-center gap-1 rounded-md border bg-muted ps-2 pe-1 text-xs">
      <span className="max-w-48 truncate">{label}</span>
      <button
        type="button"
        aria-label={`Remove ${label}`}
        onClick={onRemove}
        className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
      >
        <XIcon className="size-3" />
      </button>
    </span>
  );
}
