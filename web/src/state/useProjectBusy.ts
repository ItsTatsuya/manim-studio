import { useStudio } from "./studio";
import { useProjectDoc } from "./project";

export function useProjectBusy(projectId?: string | null) {
  const workspace = useProjectDoc((state) => state.workspace);
  return useStudio(
    (state) =>
      state.installing ||
      (!!projectId &&
        (state.threads[projectId] ?? []).some(
          (thread) =>
            thread.status === "running" &&
            (workspace === null ||
              !thread.workspace ||
              thread.workspace === workspace),
        )),
  );
}
