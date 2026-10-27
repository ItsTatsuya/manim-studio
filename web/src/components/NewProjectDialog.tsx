import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { api } from "~/lib/api";
import { styleOptions, useStyleLibrary } from "~/lib/styles";
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
import { Field } from "./ui/field";
import { Input } from "./ui/input";
import { Select } from "./ui/select";
import { toast } from "./ui/toast";

export function NewProjectDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [style, setStyle] = useState("");
  const styles = useStyleLibrary();
  useEffect(() => {
    if (open) {
      setName("");
      setStyle("");
    }
  }, [open]);
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogPopup className="max-w-sm">
        <form
          onSubmit={async (event) => {
            event.preventDefault();
            setBusy(true);
            try {
              const project = await api.createProject(
                name.trim() || "Untitled project",
                "",
                style || undefined,
              );
              await useStudio.getState().loadProjects();
              onOpenChange(false);
              void navigate({
                to: "/projects/$projectId",
                params: { projectId: project.id },
                search: {},
              });
            } catch (error) {
              toast.error(
                "Couldn't create the project",
                (error as Error).message,
              );
            } finally {
              setBusy(false);
            }
          }}
        >
          <DialogHeader>
            <DialogTitle>New project</DialogTitle>
            <DialogDescription>
              Each project keeps its own script, narration, assets and chats.
            </DialogDescription>
          </DialogHeader>
          <DialogBody>
            <Input
              aria-label="Project name"
              placeholder="e.g. Pythagorean theorem"
              value={name}
              maxLength={100}
              autoFocus
              onChange={(event) => setName(event.target.value)}
            />
            <Field label="House style" className="mt-3">
              <Select
                aria-label="House style"
                className="w-full text-foreground"
                value={style}
                disabled={busy}
                onChange={setStyle}
                options={styleOptions(styles, "Default")}
              />
            </Field>
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy}>
              Create project
            </Button>
          </DialogFooter>
        </form>
      </DialogPopup>
    </Dialog>
  );
}
