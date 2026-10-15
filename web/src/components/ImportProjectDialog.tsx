import { useNavigate } from "@tanstack/react-router";
import {
  ClipboardCopyIcon,
  FileArchiveIcon,
  FolderInputIcon,
} from "lucide-react";
import { useRef, useState } from "react";

import { api } from "~/lib/api";
import { EXTERNAL_AI_PROMPT } from "~/lib/prompts";
import { useStudio } from "~/state/studio";
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
import { Spinner } from "./ui/input";
import { toast } from "./ui/toast";

/** Keep only what a Studio project uses: scripts, narration, metadata and assets. */
function packageFiles(files: File[], folder: boolean) {
  if (!folder) return files;
  return files.filter((file) => {
    const path = file.webkitRelativePath.split("/").slice(1).join("/");
    return (
      !path.split("/").some((part) => part.startsWith(".")) &&
      (path.startsWith("assets/") ||
        path.endsWith(".py") ||
        /^(project\.json|narration\.(txt|json|md))$/.test(path))
    );
  });
}

export function ImportProjectDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const folderInput = useRef<HTMLInputElement>(null);
  const zipInput = useRef<HTMLInputElement>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const upload = async (chosen: File[], folder: boolean) => {
    const files = packageFiles(chosen, folder);
    setError(null);
    try {
      if (!files.length)
        throw new Error("Choose a folder containing a Manim Python script.");
      if (
        files.length > 2000 ||
        files.reduce((total, file) => total + file.size, 0) > 512 * 1024 ** 2
      )
        throw new Error("Keep project packages under 512 MB and 2000 files.");
      setBusy(
        folder ? chosen[0].webkitRelativePath.split("/")[0] : chosen[0].name,
      );
      const form = new FormData();
      for (const file of files)
        form.append(
          "files",
          file,
          folder ? file.webkitRelativePath : file.name,
        );
      const project = await api.importProject(form);
      await useStudio.getState().loadProjects();
      onOpenChange(false);
      for (const warning of project.warnings ?? []) toast(warning);
      toast.success(`Imported “${project.name}”`);
      void navigate({
        to: "/projects/$projectId",
        params: { projectId: project.id },
        search: {},
      });
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !busy && onOpenChange(next)}>
      <DialogPopup showClose={!busy}>
        <DialogHeader>
          <DialogTitle>Import a project</DialogTitle>
          <DialogDescription>
            Choose a folder or ZIP with a Manim script, and optionally
            narration.txt and an assets folder. Imported scripts never install
            packages.
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-3">
          <div className="grid grid-cols-2 gap-2">
            <Button
              variant="outline"
              size="lg"
              disabled={!!busy}
              onClick={() => folderInput.current?.click()}
            >
              <FolderInputIcon /> Choose folder
            </Button>
            <Button
              variant="outline"
              size="lg"
              disabled={!!busy}
              onClick={() => zipInput.current?.click()}
            >
              <FileArchiveIcon /> Choose ZIP
            </Button>
          </div>
          {busy && (
            <div
              className="flex items-center gap-2 text-muted-foreground text-sm"
              role="status"
            >
              <Spinner /> Importing {busy}…
            </div>
          )}
          {error && (
            <p
              role="alert"
              className="rounded-md border border-destructive/30 bg-destructive/10 p-2.5 text-destructive-foreground text-sm"
            >
              {error}
            </p>
          )}
          <div className="rounded-lg border bg-muted/50 p-3 text-muted-foreground text-xs leading-relaxed">
            Using an AI chat outside Studio? Copy a prompt that asks it for a
            ready-to-import package.
            <Button
              size="xs"
              variant="outline"
              className="mt-2 flex"
              onClick={() =>
                void navigator.clipboard
                  .writeText(EXTERNAL_AI_PROMPT)
                  .then(() =>
                    toast.success(
                      "Prompt copied",
                      "Add your video details at the end.",
                    ),
                  )
              }
            >
              <ClipboardCopyIcon /> Copy project prompt
            </Button>
          </div>
          <input
            ref={folderInput}
            type="file"
            hidden
            multiple
            {...{ webkitdirectory: "" }}
            onChange={(event) => {
              const files = [...(event.target.files ?? [])];
              event.target.value = "";
              if (files.length) void upload(files, true);
            }}
          />
          <input
            ref={zipInput}
            type="file"
            hidden
            accept=".zip,application/zip"
            onChange={(event) => {
              const files = [...(event.target.files ?? [])];
              event.target.value = "";
              if (files.length) void upload(files, false);
            }}
          />
        </DialogBody>
        <DialogFooter>
          <Button
            variant="ghost"
            disabled={!!busy}
            onClick={() => onOpenChange(false)}
          >
            Close
          </Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}
