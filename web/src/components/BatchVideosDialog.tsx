import { Link } from "@tanstack/react-router";
import {
  FileTextIcon,
  PlusIcon,
  XIcon,
  FolderIcon,
  FoldersIcon,
  FolderOpenIcon,
} from "lucide-react";
import { useRef, useState } from "react";
import { api } from "~/lib/api";
import { styleOptions, useStyleLibrary } from "~/lib/styles";
import { useStudio } from "~/state/studio";
import {
  ModelPicker,
  defaultChoice,
  providerReadiness,
} from "./chat/ModelPicker";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "./ui/dialog";
import { Input, Spinner } from "./ui/input";
import { Select } from "./ui/select";
import { Switch } from "./ui/switch";
import { Field } from "./ui/field";
import { cn } from "~/lib/utils";

interface VideoFile {
  file: File;
  status: string;
  project?: string;
  thread?: string;
  started?: boolean;
  error?: string;
  workspace?: string | null;
}

export function BatchVideosDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [files, setFiles] = useState<VideoFile[]>([]);
  const [choice, setChoice] = useState(defaultChoice);
  const [busy, setBusy] = useState(false);
  const [destination, setDestination] = useState<
    "same" | "separate" | "existing"
  >("same");
  const [folderName, setFolderName] = useState("");
  const [existingFolder, setExistingFolder] = useState("");
  const [sharedFolder, setSharedFolder] = useState<string | null>(null);
  const [inputError, setInputError] = useState("");
  const [style, setStyle] = useState("");
  const styles = useStyleLibrary();
  // Plan first: each chat writes a storyboard for review instead of a video.
  const [planFirst, setPlanFirst] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const projects = useStudio((state) => state.projects);
  const folderSupport = useStudio(
    (state) => state.health?.video_workspaces === true,
  );
  const providers = useStudio((state) => state.agents?.providers);
  const autoRender = useStudio((state) => state.agents?.settings.auto_render);
  const readiness = providerReadiness(
    providers?.find((provider) => provider.id === choice.provider),
  );
  const pending = files.filter((item) => !item.started);
  const update = (file: File, changes: Partial<VideoFile>) =>
    setFiles((current) =>
      current.map((item) =>
        item.file === file ? { ...item, ...changes } : item,
      ),
    );
  const start = async () => {
    setBusy(true);
    setInputError("");
    let parent = destination === "existing" ? existingFolder : sharedFolder;
    if (destination === "same" && !parent) {
      try {
        const project = await api.createProject(
          folderName.trim(),
          "",
          style || undefined,
        );
        parent = project.id;
        setSharedFolder(parent);
        await useStudio.getState().loadProjects();
      } catch (error) {
        setInputError((error as Error).message);
        setBusy(false);
        return;
      }
    }
    // Project files are isolated; provider turns may run concurrently across them.
    // Submit each one independently so a bad PDF doesn't discard other videos.
    for (const item of pending) {
      let projectId = item.project;
      let workspace = item.workspace;
      try {
        update(item.file, { status: "Reading PDF…", error: undefined });
        if (!projectId) {
          const project = await api.projectFromPdf(
            item.file,
            destination === "separate" ? undefined : parent!,
          );
          projectId = project.id;
          workspace = project.workspace;
          // Videos in a new shared folder follow its style; others choose it here.
          if (style && (destination !== "same" || !workspace))
            await api.updateProject(projectId, { style }, workspace);
          update(item.file, { project: projectId, workspace });
          await useStudio.getState().loadProjects();
        }
        update(item.file, { status: "Starting chat…" });
        const thread = item.thread
          ? await api.thread(projectId, item.thread)
          : await api.createThread(projectId, {
              ...choice,
              workspace,
              title: item.file.name.replace(/\.pdf$/i, "").slice(0, 80),
            });
        update(item.file, { thread: thread.id });
        useStudio.getState().upsertThreadSummary(thread);
        useStudio.setState((state) => ({
          details: { ...state.details, [thread.id]: thread },
        }));
        const turn = await api.sendTurn(projectId, thread.id, {
          text: planFirst
            ? "Plan this video from the supplied script. Read assets/video-script.txt (the extracted PDF text) and consult assets/video-script.pdf for layout if needed. Write the lesson storyboard in storyboard.md, keeping the script's content, examples and order."
            : "Create the video from the supplied script. Read assets/video-script.txt (the extracted PDF text) and consult assets/video-script.pdf for layout if needed. Follow its scenes, timing and narration. Write script.py and narration.txt, and check the scene before finishing. Keep the supplied narration faithful to the script.",
          context:
            'Reference files for this video:\n"assets/video-script.txt"\n"assets/video-script.pdf"',
          mode: planFirst ? "plan" : "build",
        });
        useStudio.getState().handleEvent({
          type: "thread.turn",
          project: projectId,
          thread: thread.id,
          turn,
        });
        update(item.file, {
          started: true,
          status: planFirst ? "Storyboard started" : "Chat started",
        });
      } catch (error) {
        update(item.file, {
          status: "Needs attention",
          error: (error as Error).message,
        });
      }
    }
    setBusy(false);
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!busy) onOpenChange(value);
      }}
    >
      <DialogPopup
        className="max-h-[calc(100dvh-2rem)] max-w-[36rem]"
        showClose={!busy}
      >
        <DialogHeader className="px-6 pt-6 pb-5">
          <DialogTitle>Videos from PDFs</DialogTitle>
          <DialogDescription>
            Add one script PDF per video. Each gets a chat with its own video
            script and narration.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-5 px-6 pb-6">
          <fieldset disabled={busy || files.some((item) => !!item.project)}>
            <legend className="mb-2 text-sm font-medium">Save videos to</legend>
            <div className="grid gap-2 sm:grid-cols-3">
              {[
                {
                  id: "same",
                  title: "Same folder",
                  detail: "One new project",
                  icon: FolderIcon,
                },
                {
                  id: "separate",
                  title: "Separate folders",
                  detail: "One project per PDF",
                  icon: FoldersIcon,
                },
                {
                  id: "existing",
                  title: "Existing folder",
                  detail: "Add to a project",
                  icon: FolderOpenIcon,
                },
              ].map((option) => (
                <label
                  key={option.id}
                  className={cn(
                    "relative flex cursor-pointer items-start gap-3 rounded-xl border p-3 text-sm transition-colors hover:bg-accent has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-ring sm:flex-col sm:gap-2",
                    destination === option.id
                      ? "border-primary bg-primary/10"
                      : "border-border bg-card/40",
                  )}
                >
                  <input
                    type="radio"
                    name="pdf-destination"
                    value={option.id}
                    checked={destination === option.id}
                    onChange={() =>
                      setDestination(option.id as typeof destination)
                    }
                    className="absolute inset-0 z-10 m-0 size-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
                  />
                  <option.icon className="size-4 shrink-0 text-muted-foreground" />
                  <span>
                    <span className="block font-medium">{option.title}</span>
                    <span className="mt-0.5 block text-xs text-foreground/75">
                      {option.detail}
                    </span>
                  </span>
                  <span
                    aria-hidden="true"
                    className={cn(
                      "absolute end-3 top-3 size-3 rounded-full border",
                      destination === option.id
                        ? "border-primary bg-primary shadow-[inset_0_0_0_2px_var(--popover)]"
                        : "border-muted-foreground/50",
                    )}
                  />
                </label>
              ))}
            </div>
            <div className="mt-3">
              {destination === "same" ? (
                <Field label="Project folder name">
                  <Input
                    aria-label="Project folder name"
                    value={folderName}
                    maxLength={100}
                    placeholder="e.g. Calculus lessons"
                    onChange={(event) => setFolderName(event.target.value)}
                  />
                </Field>
              ) : destination === "existing" ? (
                <Field label="Project folder">
                  <Select
                    aria-label="Project folder"
                    className="w-full"
                    value={existingFolder}
                    placeholder="Choose a project…"
                    onChange={setExistingFolder}
                    options={projects.map((project) => ({
                      value: project.id,
                      label: project.name,
                    }))}
                  />
                  {!projects.length && (
                    <p className="mt-2 text-xs text-muted-foreground">
                      Create a project first, or choose Same folder.
                    </p>
                  )}
                </Field>
              ) : (
                <p className="text-xs text-muted-foreground">
                  Each PDF creates a project named after its file.
                </p>
              )}
            </div>
          </fieldset>
          <Field label="House style">
            <Select
              aria-label="House style"
              className="w-full"
              value={style}
              disabled={busy || !styles}
              onChange={setStyle}
              options={styleOptions(
                styles,
                destination === "existing" ? "Project style" : "Default",
                destination !== "existing",
              )}
            />
          </Field>
          <div className="space-y-2">
            <Button
              variant="outline"
              className="h-14 w-full justify-start gap-3 rounded-xl border-dashed px-4"
              disabled={busy || files.length >= 20}
              onClick={() => fileInput.current?.click()}
            >
              <span className="grid size-8 place-items-center rounded-lg bg-muted">
                <PlusIcon className="size-4" />
              </span>
              <span>Add PDF scripts</span>
              <span className="ms-auto text-xs font-normal text-foreground/75">
                {files.length}/20
              </span>
            </Button>
            <input
              ref={fileInput}
              type="file"
              accept=".pdf,application/pdf"
              multiple
              disabled={busy}
              aria-label="PDF scripts"
              hidden
              onChange={(event) => {
                const incoming = [...(event.target.files ?? [])];
                if (files.length + incoming.length > 20)
                  setInputError("You can add up to 20 PDFs at a time.");
                setFiles((current) =>
                  [
                    ...current,
                    ...incoming.map((file) => ({ file, status: "Ready" })),
                  ].slice(0, 20),
                );
                event.target.value = "";
              }}
            />
            <p className="text-xs text-muted-foreground">
              PDF files with selectable text · Up to 20 MB each
            </p>
          </div>
          {inputError && (
            <p role="alert" className="text-sm text-destructive-foreground">
              {inputError}
            </p>
          )}
          {!folderSupport && destination !== "separate" && (
            <p role="status" className="text-xs text-muted-foreground">
              Restart Studio to use shared or existing project folders.
            </p>
          )}
          {!!files.length && (
            <div
              className="max-h-64 space-y-2 overflow-y-auto"
              aria-live="polite"
            >
              {files.map((item, index) => (
                <div
                  key={index}
                  className="flex items-start gap-3 rounded-lg border p-3"
                >
                  <FileTextIcon className="mt-1 size-4 shrink-0 text-muted-foreground" />
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                      {item.file.name}
                    </p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {item.status}
                    </p>
                    {item.error && (
                      <p
                        role="alert"
                        className="mt-1 text-xs text-destructive-foreground"
                      >
                        {item.error}
                      </p>
                    )}
                    {item.project && (
                      <Link
                        to="/projects/$projectId"
                        params={{ projectId: item.project }}
                        search={item.thread ? { thread: item.thread } : {}}
                        onClick={() => onOpenChange(false)}
                        className="mt-1 inline-block py-1 text-xs text-primary underline"
                      >
                        Open video chat
                      </Link>
                    )}
                  </div>
                  {!busy && (
                    <Button
                      size="icon-xs"
                      variant="ghost-muted"
                      aria-label={`Remove ${item.file.name}`}
                      onClick={() =>
                        setFiles((current) =>
                          current.filter((entry) => entry !== item),
                        )
                      }
                    >
                      <XIcon />
                    </Button>
                  )}
                </div>
              ))}
            </div>
          )}
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-xl border bg-card/40 p-3">
            <span className="text-sm font-medium">Coding assistant</span>
            <ModelPicker value={choice} onChange={setChoice} disabled={busy} />
          </div>
          <div className="flex items-center justify-between gap-3 px-1">
            <span className="text-sm font-medium">
              Plan first, review before generating
            </span>
            <Switch
              aria-label="Plan first, review before generating"
              checked={planFirst}
              disabled={busy}
              onCheckedChange={setPlanFirst}
            />
          </div>
          <p className="text-xs text-muted-foreground">
            {planFirst
              ? "Each chat writes a lesson storyboard. Nothing renders until you approve it in the Storyboard tab."
              : autoRender
                ? "Finished code is added to the render queue automatically. Videos render one at a time."
                : "Automatic rendering is off. Open each video and click Render to add it to the queue."}
          </p>
          {!readiness.ready && (
            <p className="text-xs text-muted-foreground">
              {readiness.label} · Check Settings → Providers before starting.
            </p>
          )}
        </DialogBody>
        <DialogFooter className="px-6 py-4">
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => onOpenChange(false)}
          >
            Close
          </Button>
          <Button
            disabled={
              busy ||
              !pending.length ||
              !readiness.ready ||
              (!folderSupport && destination !== "separate") ||
              (destination === "same" && !folderName.trim()) ||
              (destination === "existing" && !existingFolder)
            }
            onClick={() => void start()}
          >
            {busy && <Spinner />}
            {busy
              ? "Starting videos…"
              : pending.length
                ? planFirst
                  ? `Plan ${pending.length} video${pending.length === 1 ? "" : "s"}`
                  : `Create and start ${pending.length} video${pending.length === 1 ? "" : "s"}`
                : "Create videos"}
          </Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}
