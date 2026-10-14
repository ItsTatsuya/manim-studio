import { useState } from "react";
import { Textarea } from "../ui/input";
import { DEFAULT_VOICE, VoiceControls } from "../VoiceControls";
import { SettingsHeading } from "./SettingsLayout";

export function NarrationSettingsPage() {
  const [value, setValue] = useState({
    ...DEFAULT_VOICE,
    text: "Let's explore the idea, one step at a time. Watch how the shapes move together. Now, take a closer look.",
  });
  return (
    <>
      <SettingsHeading title="Narration">
        Try the local Kokoro voices. Choose the voice and script for each
        project in its Narration panel.
      </SettingsHeading>
      <div className="space-y-5 rounded-xl border bg-card p-5">
        <label className="block space-y-2 text-sm">
          Preview text
          <Textarea
            aria-label="Preview text"
            value={value.text}
            onChange={(event) =>
              setValue({ ...value, text: event.target.value })
            }
          />
        </label>
        <VoiceControls
          value={value}
          onChange={(changes) =>
            setValue((current) => ({ ...current, ...changes }))
          }
        />
      </div>
    </>
  );
}
