import { useEffect } from "react";

import { activeRender, renderProject } from "~/lib/render";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { toast } from "./ui/toast";

const RENDERED_FILES = /^(script\.py|narration\.txt|assets\/)/;

/** After an agent finishes editing a project, refresh the editor and render the result. */
export function AutoRender() {
  const completed = useStudio((state) => state.completedTurn);

  useEffect(() => {
    if (!completed) return;
    const { turn, project } = completed;
    useStudio.setState({ completedTurn: null });
    void (async () => {
      const doc = useProjectDoc.getState();
      if (doc.id === project && turn.changed.length)
        await doc.reload().catch(() => undefined);
      const studio = useStudio.getState();
      if (
        turn.status !== "completed" ||
        !studio.agents?.settings.auto_render ||
        !turn.changed.some((file) => RENDERED_FILES.test(file)) ||
        activeRender(studio.renders)
      )
        return;
      try {
        await renderProject(project);
      } catch (error) {
        toast.error(
          "Couldn't render the agent's changes",
          (error as Error).message,
        );
      }
    })();
  }, [completed]);

  return null;
}
