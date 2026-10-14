import { useStudio } from "~/state/studio";
import { normalizeTheme } from "~/lib/theme";
import { OUTPUT_PRESETS, choosePreset, currentPreset } from "~/lib/render";
import { Select } from "../ui/select";
import { Switch } from "../ui/switch";
import { api } from "~/lib/api";
import { toast } from "../ui/toast";
import { Button } from "../ui/button";
import { SettingsHeading, SettingsRow } from "./SettingsLayout";

export function GeneralSettings() {
  const prefs = useStudio((state) => state.prefs);
  const agents = useStudio((state) => state.agents);
  const save = async (auto_render: boolean) => {
    try {
      await api.saveAgentSettings({ auto_render });
      await useStudio.getState().loadAgents();
    } catch (error) {
      toast.error("Couldn't save settings", (error as Error).message);
    }
  };
  return (
    <>
      <SettingsHeading title="General">
        Make Studio fit your workflow.
      </SettingsHeading>
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
        description="Start a render when an agent successfully changes the script, narration or assets."
      >
        <Switch
          aria-label="Render after generation"
          checked={agents?.settings.auto_render ?? true}
          disabled={!agents}
          onCheckedChange={(value) => void save(value)}
        />
      </SettingsRow>
      <div className="mt-6 rounded-xl border bg-muted/50 p-4 text-sm">
        <h2 className="font-medium">Keyboard shortcuts</h2>
        <div className="mt-2 space-y-1 text-xs text-muted-foreground">
          <p>Enter sends a chat message · Shift+Enter inserts a new line</p>
          <p>
            Ctrl+Enter renders the current scene · Ctrl+F searches in the script
            editor
          </p>
          <p>Tab indents Python · Escape, then Tab leaves the editor</p>
        </div>
      </div>
    </>
  );
}
