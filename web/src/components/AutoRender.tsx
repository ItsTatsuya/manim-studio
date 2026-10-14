import { useEffect, useRef } from "react";

import { activeRender, renderProject } from "~/lib/render";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { toast } from "./ui/toast";

const RENDERED_FILES = /^(script\.py|narration\.txt|assets\/)/;

/**
 * After an agent finishes editing a project, refresh the editor and render the
 * result. A render already in progress delays it instead of silently skipping,
 * and a missing narration voice renders silently with a clear notice.
 */
export function AutoRender() {
  const completed = useStudio((state) => state.completedTurn);
  const renders = useStudio((state) => state.renders);
  const pending = useRef<string | null>(null);
  const starting = useRef(false);

  const start = async (project: string) => {
    if (starting.current) return;
    starting.current = true;
    try {
      const studio = useStudio.getState();
      const wantsVoice = (() => {
        const doc = useProjectDoc.getState();
        const narration =
          doc.id === project
            ? doc.narration
            : studio.projects.find((item) => item.id === project)?.narration;
        return !!narration?.enabled && !!narration.text.trim();
      })();
      const quality = useProjectDoc.getState().narration?.quality ?? "standard";
      const voiceReady = studio.narration?.models[quality]?.installed ?? true;
      const silent = wantsVoice && !voiceReady;
      toast("Rendering your animation…", {
        description: silent
          ? "The narration voice isn't downloaded yet, so this render has no narration. Download it in the Narration tab."
          : "Studio is rendering the agent's changes.",
      });
      await renderProject(project, { narration: !silent });
    } catch (error) {
      toast.error(
        "Couldn't render the agent's changes",
        (error as Error).message,
      );
    } finally {
      starting.current = false;
    }
  };

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
        !turn.changed.some((file) => RENDERED_FILES.test(file))
      )
        return;
      if (activeRender(studio.renders)) {
        // Wait for the running render, then render the new version.
        pending.current = project;
        return;
      }
      await start(project);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [completed]);

  useEffect(() => {
    if (!pending.current || activeRender(renders)) return;
    const project = pending.current;
    pending.current = null;
    void start(project);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [renders]);

  return null;
}
