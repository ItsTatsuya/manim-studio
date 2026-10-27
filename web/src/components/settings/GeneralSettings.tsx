import { useStudio } from "~/state/studio";
import { normalizeTheme } from "~/lib/theme";
import { OUTPUT_PRESETS, choosePreset, currentPreset } from "~/lib/render";
import { Select } from "../ui/select";
import { Switch } from "../ui/switch";
import { api } from "~/lib/api";
import { toast } from "../ui/toast";
import { Button } from "../ui/button";
import { Kbd } from "../ui/input";
import { QUEUE_ERROR_OPTIONS, QUEUE_START_OPTIONS } from "~/lib/queue";
import { SettingsRow, SettingsSection } from "./SettingsLayout";
import { WatermarkSettingsSection } from "./WatermarkSettingsSection";

const SHORTCUTS: { label: string; keys: string[][] }[] = [
  { label: "Send a chat message", keys: [["Enter"]] },
  { label: "New line in a message", keys: [["Shift", "Enter"]] },
  { label: "Render the current scene", keys: [["Ctrl", "Enter"]] },
  { label: "Search in the script editor", keys: [["Ctrl", "F"]] },
  { label: "Indent Python", keys: [["Tab"]] },
  { label: "Leave the script editor", keys: [["Esc"], ["Tab"]] },
];

export function GeneralSettings() {
  const prefs = useStudio((state) => state.prefs);
  const agents = useStudio((state) => state.agents);
  const save = async (changes: Parameters<typeof api.saveAgentSettings>[0]) => {
    try {
      await api.saveAgentSettings(changes);
      await useStudio.getState().loadAgents();
    } catch (error) {
      toast.error("Couldn't save settings", (error as Error).message);
    }
  };
  return (
    <>
      <SettingsSection title="Workspace">
        <SettingsRow
          title="Setup & quick guide"
          description="Connect an assistant or revisit how Studio works."
        >
          <Button
            variant="outline"
            onClick={() => window.dispatchEvent(new Event("studio:setup"))}
          >
            Open setup
          </Button>
        </SettingsRow>
        <SettingsRow
          title="Appearance"
          description="Follow your system or choose a theme."
        >
          <Select
            aria-label="Appearance"
            value={normalizeTheme(prefs["manim-theme"])}
            onChange={(value) =>
              useStudio.getState().setPref("manim-theme", value)
            }
            options={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection title="Rendering">
        <SettingsRow
          title="Default video output"
          description="You can change this in the Preview panel."
        >
          <Select
            aria-label="Default video output"
            value={currentPreset(prefs).id}
            onChange={(id) =>
              choosePreset(OUTPUT_PRESETS.find((preset) => preset.id === id)!)
            }
            options={OUTPUT_PRESETS.map((preset) => ({
              value: preset.id,
              label: `${preset.label} · ${preset.detail}`,
            }))}
          />
        </SettingsRow>
        <SettingsRow
          title="Render after generation"
          description="Render the agent's changes, fix script errors in the same chat, and retry automatically (up to 3 repairs)."
        >
          <Switch
            aria-label="Render after generation"
            checked={agents?.settings.auto_render ?? true}
            disabled={!agents}
            onCheckedChange={(value) => void save({ auto_render: value })}
          />
        </SettingsRow>
        <SettingsRow
          title="Start the render queue"
          description="Videos render one at a time. Choose whether they begin on their own or wait until you start the queue."
        >
          <Select
            aria-label="Start the render queue"
            value={agents?.settings.queue_start ?? "auto"}
            disabled={!agents}
            options={QUEUE_START_OPTIONS}
            onChange={(queue_start) => void save({ queue_start })}
          />
        </SettingsRow>
        <SettingsRow
          title="When a video fails"
          description="Fix it with the agent and render it again, skip it, or pause the queue. The rest of the queue keeps going while a video is fixed."
        >
          <Select
            aria-label="When a video fails"
            value={agents?.settings.queue_on_error ?? "fix"}
            disabled={!agents}
            options={QUEUE_ERROR_OPTIONS}
            onChange={(queue_on_error) => void save({ queue_on_error })}
          />
        </SettingsRow>
      </SettingsSection>

      <WatermarkSettingsSection />

      <SettingsSection title="Keyboard shortcuts">
        {SHORTCUTS.map((shortcut) => (
          <SettingsRow key={shortcut.label} title={shortcut.label}>
            <span className="flex items-center gap-1.5">
              {shortcut.keys.map((combo, index) => (
                <span key={combo.join("+")} className="flex items-center gap-1">
                  {index > 0 && (
                    <span className="text-muted-foreground text-xs">then</span>
                  )}
                  {combo.map((key) => (
                    <Kbd key={key}>{key}</Kbd>
                  ))}
                </span>
              ))}
            </span>
          </SettingsRow>
        ))}
      </SettingsSection>
    </>
  );
}
