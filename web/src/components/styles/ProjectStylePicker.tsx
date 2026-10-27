import { Link } from "@tanstack/react-router";
import { useState } from "react";

import { api } from "~/lib/api";
import { styleById, styleOptions, useStyleLibrary } from "~/lib/styles";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { Select } from "../ui/select";
import { toast } from "../ui/toast";
import { StyleChips } from "./StyleSwatch";

/** Choose the house style the open project (or batch video) uses. */
export function ProjectStylePicker({ projectId }: { projectId: string }) {
  const library = useStyleLibrary();
  const project = useProjectDoc((state) => state.project);
  const workspace = useProjectDoc((state) => state.workspace);
  const [busy, setBusy] = useState(false);
  const value = project?.style ?? "";
  const shown = value ? styleById(library, value) : undefined;
  const choose = async (style: string) => {
    if (!project || style === value) return;
    setBusy(true);
    try {
      await api.updateProject(projectId, { style }, workspace);
      const current = useProjectDoc.getState();
      if (current.id === projectId && current.project)
        useProjectDoc.setState({
          project: { ...current.project, style: style || undefined },
        });
      await useStudio.getState().loadProjects();
    } catch (error) {
      toast.error("Couldn't change the style", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex min-w-0 flex-col gap-1.5 text-muted-foreground text-xs">
      <span className="flex items-center justify-between gap-2">
        <span>House style</span>
        <Link
          to="/settings/styles"
          className="text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
        >
          Manage
        </Link>
      </span>
      <div className="flex min-w-0 items-center gap-2">
        <Select
          aria-label="House style"
          size="sm"
          className="min-w-0 flex-1 text-foreground"
          value={value}
          disabled={!project || busy}
          onChange={(style) => void choose(style)}
          options={styleOptions(
            library,
            workspace ? "Project style" : "Default",
            !workspace,
          )}
        />
        {shown && <StyleChips style={shown} />}
      </div>
    </div>
  );
}
