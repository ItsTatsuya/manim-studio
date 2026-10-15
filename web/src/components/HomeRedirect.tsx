import { Navigate } from "@tanstack/react-router";
import { FolderPlusIcon, UploadIcon } from "lucide-react";
import { useState } from "react";

import { useStudio } from "~/state/studio";
import { StudioMark } from "./icons";
import { ImportProjectDialog } from "./ImportProjectDialog";
import { NewProjectDialog } from "./NewProjectDialog";
import { Button } from "./ui/button";

/** Open the last project; with none, offer to start one. */
export function HomeRedirect() {
  const projects = useStudio((state) => state.projects);
  const last = useStudio((state) => state.prefs["manim-project"]);
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  const target = projects.find((project) => project.id === last) ?? projects[0];
  if (target)
    return (
      <Navigate
        to="/projects/$projectId"
        params={{ projectId: target.id }}
        replace
      />
    );
  return (
    <div className="grid h-full place-items-center p-8">
      <div className="max-w-md text-center">
        <StudioMark className="mx-auto mb-5 size-12" />
        <h1 className="font-semibold text-2xl tracking-tight">
          Make your first animation
        </h1>
        <p className="mt-2 text-muted-foreground">
          Describe a video and let Codex or Claude write the Manim scene,
          narration and assets.
        </p>
        <div className="mt-6 flex justify-center gap-2">
          <Button onClick={() => setCreating(true)}>
            <FolderPlusIcon /> Create project
          </Button>
          <Button variant="outline" onClick={() => setImporting(true)}>
            <UploadIcon /> Import project
          </Button>
        </div>
      </div>
      <NewProjectDialog open={creating} onOpenChange={setCreating} />
      <ImportProjectDialog open={importing} onOpenChange={setImporting} />
    </div>
  );
}
