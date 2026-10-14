import { api, type UpdateState } from "~/lib/api";
import { useProjectDoc } from "~/state/project";
import { useStudio } from "~/state/studio";
import { confirmAction } from "~/components/ui/dialog";
import { toast } from "~/components/ui/toast";

export type UpdateAction = "check" | "download" | "cancel" | "install";

/** Whether work is running that an update restart would interrupt. */
export function isBusy() {
  const state = useStudio.getState();
  return (
    state.renders.some((render) =>
      ["queued", "rendering"].includes(render.status),
    ) ||
    Object.values(state.threads).some((threads) =>
      threads.some((thread) => thread.status === "running"),
    )
  );
}

/** Run one updater action with the same confirmations everywhere. */
export async function runUpdateAction(
  kind: UpdateAction,
): Promise<UpdateState | null> {
  if (
    kind === "install" &&
    !(await confirmAction({
      title: "Install update and restart?",
      description:
        "Studio will close and reopen. Your projects, assets and render history are preserved.",
      confirm: "Install and restart",
    }))
  )
    return null;
  try {
    if (kind === "install") {
      if (
        !(await useProjectDoc.getState().flush()) ||
        !(await useStudio.getState().flushPrefs())
      )
        throw new Error(
          "Your changes couldn't be saved. Retry saving before updating.",
        );
      useStudio.setState({ installing: true });
    }
    const state = await {
      check: api.checkUpdates,
      download: api.downloadUpdate,
      cancel: api.cancelUpdate,
      install: api.installUpdate,
    }[kind]();
    useStudio.getState().setUpdates(state);
    if (kind === "install" && state.status !== "installing")
      useStudio.setState({ installing: false });
    return state;
  } catch (error) {
    useStudio.setState({ installing: false });
    toast.error("Update action failed", (error as Error).message);
    return null;
  }
}
