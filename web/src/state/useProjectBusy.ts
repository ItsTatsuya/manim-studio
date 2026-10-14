import { useStudio } from "./studio";

export function useProjectBusy(projectId?: string | null) {
  return useStudio(
    (state) =>
      state.installing ||
      (!!projectId &&
        (state.threads[projectId] ?? []).some(
          (thread) => thread.status === "running",
        )),
  );
}
