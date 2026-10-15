import { useState } from "react";
import { Field } from "../ui/field";
import { Textarea } from "../ui/input";
import { DEFAULT_VOICE, VoiceControls } from "../VoiceControls";
import { SettingsSection } from "./SettingsLayout";

export function NarrationSettingsPage() {
  const [value, setValue] = useState({
    ...DEFAULT_VOICE,
    text: "Let's explore the idea, one step at a time. Watch how the shapes move together. Now, take a closer look.",
  });
  return (
    <SettingsSection
      title="Voice preview"
      description="Try the local Kokoro voices. Choose the voice and script for each project in its Narration panel."
    >
      <div className="space-y-5 p-4">
        <Field label="Preview text">
          <Textarea
            aria-label="Preview text"
            value={value.text}
            onChange={(event) =>
              setValue({ ...value, text: event.target.value })
            }
          />
        </Field>
        <VoiceControls
          value={value}
          onChange={(changes) =>
            setValue((current) => ({ ...current, ...changes }))
          }
        />
      </div>
    </SettingsSection>
  );
}
