import { useEffect, useState } from "react";

import {
  watermarkApi,
  watermarkProblem,
  type WatermarkSettings,
} from "~/lib/watermark";
import { Button } from "../ui/button";
import { Spinner } from "../ui/input";
import { toast } from "../ui/toast";
import { WatermarkEditor } from "../watermark/WatermarkEditor";
import { SettingsSection } from "./SettingsLayout";

/** The default watermark for every render; projects can override it. */
export function WatermarkSettingsSection() {
  const [saved, setSaved] = useState<WatermarkSettings | null>(null);
  const [draft, setDraft] = useState<WatermarkSettings | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let active = true;
    watermarkApi
      .defaults()
      .then(({ settings }) => {
        if (!active) return;
        setSaved(settings);
        setDraft(settings);
      })
      .catch((error: Error) =>
        toast.error("Couldn't load the watermark", error.message),
      );
    return () => {
      active = false;
    };
  }, []);
  const changed = JSON.stringify(saved) !== JSON.stringify(draft);
  const problem = draft ? watermarkProblem(draft) : null;
  const save = async () => {
    if (!draft || problem) return;
    setSaving(true);
    try {
      const { settings } = await watermarkApi.saveDefaults(draft);
      setSaved(settings);
      setDraft(settings);
      toast.success("Watermark saved");
    } catch (error) {
      toast.error("Couldn't save the watermark", (error as Error).message);
    } finally {
      setSaving(false);
    }
  };
  return (
    <SettingsSection
      title="Watermark"
      description="Your logo or text on every rendered video. Projects and batch videos can use their own."
      action={
        <div className="flex items-center gap-1.5">
          {changed && (
            <Button
              size="xs"
              variant="ghost"
              disabled={saving}
              onClick={() => setDraft(saved)}
            >
              Discard
            </Button>
          )}
          <Button
            size="xs"
            disabled={!changed || !!problem || saving}
            onClick={() => void save()}
          >
            Save watermark
          </Button>
        </div>
      }
    >
      <div className="p-4">
        {draft ? (
          <>
            <WatermarkEditor value={draft} onChange={setDraft} />
            {problem && changed && (
              <p
                role="alert"
                className="mt-3 text-destructive-foreground text-xs"
              >
                {problem}
              </p>
            )}
          </>
        ) : (
          <div className="grid place-items-center py-6">
            <Spinner />
          </div>
        )}
      </div>
    </SettingsSection>
  );
}
