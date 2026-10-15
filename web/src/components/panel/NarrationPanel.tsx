import { useProjectDoc } from "~/state/project";
import { useProjectBusy } from "~/state/useProjectBusy";
import { Field } from "../ui/field";
import { Textarea } from "../ui/input";
import { Switch } from "../ui/switch";
import { VoiceControls } from "../VoiceControls";

export function NarrationPanel() {
  const narration = useProjectDoc((state) => state.narration);
  const id = useProjectDoc((state) => state.id);
  const busy = useProjectBusy(id);
  if (!narration) return null;
  const change = useProjectDoc.getState().setNarration;
  return (
    <fieldset
      disabled={busy}
      className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4"
    >
      <div className="flex items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-medium">Voice narration</h2>
          <p className="mt-1 text-xs text-muted-foreground">
            Saved in narration.txt and mixed into your video.
          </p>
        </div>
        <Switch
          disabled={busy}
          aria-label="Enable narration"
          checked={narration.enabled}
          onCheckedChange={(enabled) => change({ enabled })}
        />
      </div>
      <Field label="Narration script" className="min-h-40 flex-1">
        <Textarea
          aria-label="Narration script"
          className="min-h-40 flex-1"
          value={narration.text}
          placeholder="Write your narration, or ask the agent to generate it…"
          onChange={(event) => change({ text: event.target.value })}
        />
      </Field>
      <VoiceControls value={narration} onChange={change} />
    </fieldset>
  );
}
