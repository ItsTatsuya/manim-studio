import { StampIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";

import {
  describeWatermark,
  watermarkApi,
  watermarkProblem,
  type ProjectWatermark,
  type WatermarkSettings,
} from "~/lib/watermark";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogBody,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "../ui/dialog";
import { Spinner } from "../ui/input";
import { Switch } from "../ui/switch";
import { WatermarkEditor, WatermarkPreview } from "./WatermarkEditor";

function inheritedLabel(state: ProjectWatermark, workspace?: string | null) {
  return state.inherited_from === "project" && workspace
    ? "the project's watermark"
    : "the default watermark from Settings";
}

/** The project's (or batch video's) watermark override, in a dialog. */
export function ProjectWatermarkDialog({
  open,
  onOpenChange,
  projectId,
  workspace,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectId: string;
  workspace?: string | null;
  onSaved?: (state: ProjectWatermark) => void;
}) {
  const [state, setState] = useState<ProjectWatermark | null>(null);
  const [custom, setCustom] = useState(false);
  const [draft, setDraft] = useState<WatermarkSettings | null>(null);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!open) return;
    let active = true;
    setState(null);
    setError("");
    watermarkApi
      .project(projectId, workspace)
      .then((loaded) => {
        if (!active) return;
        setState(loaded);
        setCustom(loaded.own !== null);
        setDraft(loaded.own ?? loaded.inherited);
      })
      .catch((failure: Error) => {
        if (active) setError(failure.message);
      });
    return () => {
      active = false;
    };
  }, [open, projectId, workspace]);
  const problem = custom && draft ? watermarkProblem(draft) : null;
  const save = async () => {
    if (!draft || problem) return;
    setSaving(true);
    setError("");
    try {
      const saved = await watermarkApi.saveProject(
        projectId,
        custom ? draft : null,
        workspace,
      );
      onSaved?.(saved);
      onOpenChange(false);
    } catch (failure) {
      setError((failure as Error).message);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogPopup className="max-w-2xl">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {workspace ? "Video watermark" : "Project watermark"}
            </DialogTitle>
          </DialogHeader>
          <DialogBody>
            {!state || !draft ? (
              error ? (
                <p role="alert" className="text-destructive-foreground text-sm">
                  {error}
                </p>
              ) : (
                <div className="grid place-items-center py-10">
                  <Spinner />
                </div>
              )
            ) : (
              <>
                <div className="mb-4 flex items-center justify-between gap-4 rounded-lg border p-3">
                  <span className="min-w-0">
                    <span className="block font-medium text-sm">
                      Use a custom watermark
                    </span>
                    <span className="block text-muted-foreground text-xs">
                      Off uses {inheritedLabel(state, workspace)} (
                      {describeWatermark(state.inherited)}).
                    </span>
                  </span>
                  <Switch
                    aria-label="Use a custom watermark"
                    checked={custom}
                    onCheckedChange={(value) => {
                      setCustom(value);
                      if (value && !state.own) setDraft(state.inherited);
                    }}
                  />
                </div>
                {custom ? (
                  <WatermarkEditor
                    value={draft}
                    onChange={setDraft}
                    projectId={projectId}
                    workspace={workspace}
                  />
                ) : (
                  <WatermarkPreview
                    settings={state.inherited}
                    className="mx-auto max-w-sm"
                  />
                )}
                {(problem || error) && (
                  <p
                    role="alert"
                    className="mt-3 text-destructive-foreground text-xs"
                  >
                    {problem || error}
                  </p>
                )}
              </>
            )}
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={!state || !!problem || saving}>
              Save watermark
            </Button>
          </DialogFooter>
        </form>
      </DialogPopup>
    </Dialog>
  );
}

/** A compact button showing the watermark the next render will use. */
export function ProjectWatermarkButton({
  projectId,
  workspace,
}: {
  projectId: string;
  workspace?: string | null;
}) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<ProjectWatermark | null>(null);
  const load = useCallback(() => {
    watermarkApi
      .project(projectId, workspace)
      .then(setState)
      .catch(() => setState(null));
  }, [projectId, workspace]);
  useEffect(load, [load]);
  return (
    <>
      <Button
        size="xs"
        variant="ghost-muted"
        onClick={() => setOpen(true)}
        title="Watermark"
      >
        <StampIcon />
        <span className="truncate">
          {state ? describeWatermark(state.settings) : "Watermark"}
        </span>
      </Button>
      <ProjectWatermarkDialog
        open={open}
        onOpenChange={(value) => {
          setOpen(value);
          if (!value) load();
        }}
        projectId={projectId}
        workspace={workspace}
        onSaved={setState}
      />
    </>
  );
}
