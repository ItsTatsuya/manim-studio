import { DownloadIcon, PlayIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { api, type NarrationSettings } from "~/lib/api";
import { formatBytes } from "~/lib/utils";
import { useStudio } from "~/state/studio";
import { Button } from "./ui/button";
import { Field } from "./ui/field";
import { Input, Spinner } from "./ui/input";
import { Progress } from "./ui/progress";
import { Select } from "./ui/select";
import { toast } from "./ui/toast";

export const DEFAULT_VOICE: NarrationSettings = {
  enabled: false,
  text: "",
  voice: "af_heart",
  speed: 1,
  quality: "standard",
};

export function VoiceControls({
  value,
  onChange,
}: {
  value: NarrationSettings;
  onChange: (changes: Partial<NarrationSettings>) => void;
}) {
  const status = useStudio((state) => state.narration);
  const [busy, setBusy] = useState(false);
  const [audio, setAudio] = useState<string | null>(null);
  const [speedText, setSpeedText] = useState(String(value.speed));
  const lastSpeed = useRef(value.speed);
  // Follow external changes (loading another project) without fighting typing.
  useEffect(() => {
    if (lastSpeed.current !== value.speed) {
      lastSpeed.current = value.speed;
      if (Number(speedText) !== value.speed) setSpeedText(String(value.speed));
    }
  }, [value.speed, speedText]);
  const downloading = status?.download.status === "downloading";
  const installed = status?.models[value.quality].installed;
  useEffect(() => {
    if (!downloading) return;
    const timer = window.setInterval(
      () =>
        void useStudio
          .getState()
          .loadNarration()
          .catch(() => undefined),
      700,
    );
    return () => window.clearInterval(timer);
  }, [downloading]);
  useEffect(
    () => () => {
      if (audio) URL.revokeObjectURL(audio);
    },
    [audio],
  );

  const preview = async () => {
    setBusy(true);
    try {
      const blob = await api.previewVoice(value);
      setAudio(URL.createObjectURL(blob));
    } catch (error) {
      toast.error("Couldn't preview the voice", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const download = async () => {
    try {
      const narration = downloading
        ? await api.cancelVoiceDownload()
        : await api.downloadVoice(value.quality);
      useStudio.setState({ narration });
    } catch (error) {
      toast.error("Voice download failed", (error as Error).message);
    }
  };
  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 items-end gap-x-3 gap-y-4">
        <Field label="Voice">
          <Select
            aria-label="Voice"
            className="w-full"
            value={value.voice}
            onChange={(voice) => onChange({ voice })}
            options={(status?.voices ?? []).map((voice) => ({
              value: voice.id,
              label: voice.name,
              group: voice.language,
            }))}
          />
        </Field>
        <Field label="Model quality">
          <Select
            aria-label="Voice quality"
            className="w-full"
            value={value.quality}
            onChange={(quality) => onChange({ quality })}
            options={[
              {
                value: "standard",
                label: "Standard",
                description: "Smaller download",
              },
              {
                value: "high",
                label: "High quality",
                description: "Full precision model",
              },
            ]}
          />
        </Field>
        <Field label="Speaking speed">
          <Input
            aria-label="Speaking speed"
            type="number"
            min="0.5"
            max="2"
            step="0.05"
            value={speedText}
            onChange={(event) => {
              // Keep what is typed so partial values like "0." can be edited;
              // only a complete, in-range number is saved.
              setSpeedText(event.target.value);
              const speed = event.target.valueAsNumber;
              if (Number.isFinite(speed) && speed >= 0.5 && speed <= 2)
                onChange({ speed });
            }}
            onBlur={() => setSpeedText(String(value.speed))}
          />
        </Field>
        <Button
          variant="outline"
          className="w-full min-w-0 text-ui"
          disabled={busy || !installed}
          onClick={() => void preview()}
        >
          {busy ? <Spinner /> : <PlayIcon />}
          <span className="truncate">
            {busy ? "Preparing preview…" : "Preview voice"}
          </span>
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        1× is the natural speaking pace.
      </p>
      <p className="text-xs leading-relaxed text-muted-foreground">
        Write as you speak: short sentences, punctuation for pauses, and
        equations in words. Studio adds sentence pauses and evens out loudness.
      </p>
      {status && !installed && (
        <div className="rounded-lg border bg-muted/50 p-3 text-xs">
          <p className="tabular-nums">
            {downloading
              ? `Downloading voice model · ${formatBytes(status.download.done)} / ${formatBytes(status.download.total)}`
              : `Download the local voice model (${formatBytes(status.models[value.quality].download_size)}). It runs on your CPU.`}
          </p>
          {downloading && (
            <Progress
              aria-label="Voice download progress"
              className="mt-2"
              value={status.download.done}
              max={status.download.total || 1}
            />
          )}
          <Button
            size="xs"
            variant="outline"
            className="mt-2"
            onClick={() => void download()}
          >
            {downloading ? (
              "Cancel download"
            ) : (
              <>
                <DownloadIcon /> Download voice
              </>
            )}
          </Button>
        </div>
      )}
      {status?.download.status === "failed" && (
        <p role="alert" className="text-xs text-destructive-foreground">
          {status.download.error}
        </p>
      )}
      {audio && (
        <audio
          key={audio}
          src={audio}
          controls
          autoPlay
          aria-label="Voice preview"
          className="h-9 w-full"
        />
      )}
    </div>
  );
}
