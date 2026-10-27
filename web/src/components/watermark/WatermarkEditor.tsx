import { ImageIcon, UploadIcon } from "lucide-react";
import type * as React from "react";
import { useEffect, useRef, useState } from "react";

import { api, type Asset } from "~/lib/api";
import { cn } from "~/lib/utils";
import {
  isLogoFile,
  LOGO_ACCEPT,
  WATERMARK_POSITIONS,
  watermarkApi,
  type StoredLogo,
  type WatermarkPosition,
  type WatermarkSettings,
} from "~/lib/watermark";
import { Button } from "../ui/button";
import { Field } from "../ui/field";
import { Input, Spinner } from "../ui/input";
import { Select } from "../ui/select";
import { toast } from "../ui/toast";

/** The video frame with the watermark where the render will put it. */
export function WatermarkPreview({
  settings,
  className,
}: {
  settings: WatermarkSettings;
  className?: string;
}) {
  const margin = `${settings.margin}cqw`;
  const place: React.CSSProperties = {};
  const [vertical, horizontal] =
    settings.position === "center"
      ? ["center", "center"]
      : settings.position.split("-");
  if (vertical === "top") place.top = margin;
  else if (vertical === "bottom") place.bottom = margin;
  else place.top = "50%";
  if (horizontal === "left") place.left = margin;
  else if (horizontal === "right") place.right = margin;
  else place.left = "50%";
  place.transform = `translate(${horizontal === "center" ? "-50%" : "0"}, ${
    vertical === "center" ? "-50%" : "0"
  })`;
  const room = `calc(100cqw - 2 * ${margin})`;
  const roomHeight = `calc(100cqh - 2 * ${margin})`;
  return (
    <div
      className={cn(
        "relative aspect-video w-full overflow-hidden rounded-lg border bg-[linear-gradient(135deg,#1d2533,#0b0f17_60%,#253047)]",
        className,
      )}
      style={{ containerType: "size" }}
      data-testid="watermark-preview"
      aria-label="Watermark preview"
      role="img"
    >
      <div className="absolute inset-0 grid place-items-center text-[6cqw] font-semibold text-white/10 select-none">
        16:9
      </div>
      {settings.type === "image" && settings.logo && (
        <img
          src={watermarkApi.logoUrl(settings.logo)}
          alt=""
          className="absolute object-contain"
          style={{
            ...place,
            width: `${settings.scale}cqw`,
            maxWidth: room,
            maxHeight: roomHeight,
            opacity: settings.opacity,
          }}
        />
      )}
      {settings.type === "text" && settings.text.trim() && (
        <span
          className="absolute whitespace-nowrap font-semibold leading-none"
          style={{
            ...place,
            fontSize: `${(settings.font_size / 1080) * 100}cqh`,
            color: settings.color,
            opacity: settings.opacity,
            maxWidth: room,
          }}
        >
          {settings.text.trim()}
        </span>
      )}
    </div>
  );
}

const GRID: (WatermarkPosition | null)[] = [
  "top-left",
  null,
  "top-right",
  null,
  "center",
  null,
  "bottom-left",
  "bottom-center",
  "bottom-right",
];

function PositionPicker({
  value,
  onChange,
}: {
  value: WatermarkPosition;
  onChange: (value: WatermarkPosition) => void;
}) {
  return (
    <div
      role="radiogroup"
      aria-label="Watermark position"
      className="grid w-24 grid-cols-3 gap-1 rounded-md border bg-background p-1"
    >
      {GRID.map((position, index) =>
        position ? (
          <button
            key={position}
            type="button"
            role="radio"
            aria-checked={value === position}
            aria-label={
              WATERMARK_POSITIONS.find((item) => item.value === position)?.label
            }
            title={
              WATERMARK_POSITIONS.find((item) => item.value === position)?.label
            }
            onClick={() => onChange(position)}
            className={cn(
              "h-5 rounded-sm outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
              value === position
                ? "bg-primary"
                : "bg-muted hover:bg-accent-foreground/20",
            )}
          />
        ) : (
          <span key={`gap-${index}`} />
        ),
      )}
    </div>
  );
}

function Slider({
  label,
  value,
  min,
  max,
  step,
  format,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  format: (value: number) => string;
  onChange: (value: number) => void;
}) {
  return (
    <Field
      label={
        <span className="flex justify-between">
          <span>{label}</span>
          <span className="tabular-nums">{format(value)}</span>
        </span>
      }
    >
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
        className="h-5 w-full accent-primary"
      />
    </Field>
  );
}

/** Edit one set of watermark settings; changes are reported, not saved. */
export function WatermarkEditor({
  value,
  onChange,
  projectId,
  workspace,
}: {
  value: WatermarkSettings;
  onChange: (value: WatermarkSettings) => void;
  /** Offer the project's image assets as logos. */
  projectId?: string;
  workspace?: string | null;
}) {
  const [busy, setBusy] = useState(false);
  const [assets, setAssets] = useState<Asset[] | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const set = (changes: Partial<WatermarkSettings>) =>
    onChange({ ...value, ...changes });
  useEffect(() => {
    if (!projectId || value.type !== "image") return;
    let active = true;
    api
      .assets(projectId, workspace)
      .then(({ assets }) => {
        if (active)
          setAssets(
            assets.filter(
              (asset) =>
                (asset.kind === "image" || asset.kind === "vector") &&
                isLogoFile(asset.path),
            ),
          );
      })
      .catch(() => {
        if (active) setAssets([]);
      });
    return () => {
      active = false;
    };
  }, [projectId, workspace, value.type]);
  const applyLogo = async (load: () => Promise<StoredLogo>) => {
    setBusy(true);
    try {
      const logo = await load();
      onChange({
        ...value,
        type: "image",
        logo: logo.logo,
        logo_name: logo.name,
      });
    } catch (error) {
      toast.error("Couldn't use this logo", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <div className="grid content-start gap-3">
        <Field label="Watermark">
          <Select
            aria-label="Watermark type"
            value={value.type}
            onChange={(type) => set({ type })}
            options={[
              { value: "none", label: "None" },
              { value: "image", label: "Logo image" },
              { value: "text", label: "Text" },
            ]}
          />
        </Field>
        {value.type === "image" && (
          <>
            <div className="flex items-center gap-2">
              <div className="grid size-10 shrink-0 place-items-center overflow-hidden rounded-md border bg-muted">
                {value.logo ? (
                  <img
                    src={watermarkApi.logoUrl(value.logo)}
                    alt=""
                    className="max-h-full max-w-full object-contain"
                  />
                ) : (
                  <ImageIcon className="size-4 text-muted-foreground" />
                )}
              </div>
              <div className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
                {value.logo ? value.logo_name || "Logo" : "No logo chosen"}
              </div>
              <Button
                size="xs"
                variant="outline"
                disabled={busy}
                onClick={() => input.current?.click()}
              >
                {busy ? <Spinner className="size-3.5" /> : <UploadIcon />}
                Upload logo
              </Button>
              <input
                ref={input}
                type="file"
                accept={LOGO_ACCEPT}
                className="hidden"
                aria-label="Upload watermark logo"
                data-testid="watermark-logo-input"
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void applyLogo(() => watermarkApi.uploadLogo(file));
                }}
              />
            </div>
            {projectId && !!assets?.length && (
              <Field label="Or use a project asset">
                <Select
                  aria-label="Logo from project assets"
                  size="sm"
                  value=""
                  placeholder="Choose an image…"
                  disabled={busy}
                  onChange={(path) =>
                    void applyLogo(() =>
                      watermarkApi.logoFromAsset(projectId, path, workspace),
                    )
                  }
                  options={assets.map((asset) => ({
                    value: asset.path,
                    label: asset.path,
                  }))}
                />
              </Field>
            )}
            <Slider
              label="Size"
              value={value.scale}
              min={2}
              max={60}
              step={1}
              format={(number) => `${number}% of width`}
              onChange={(scale) => set({ scale })}
            />
          </>
        )}
        {value.type === "text" && (
          <>
            <Field label="Text">
              <Input
                aria-label="Watermark text"
                maxLength={200}
                value={value.text}
                placeholder="© Your channel"
                onChange={(event) =>
                  set({ text: event.target.value.replace(/[\r\n]+/g, " ") })
                }
              />
            </Field>
            <div className="grid grid-cols-[1fr_auto] items-end gap-3">
              <Slider
                label="Font size"
                value={value.font_size}
                min={12}
                max={160}
                step={1}
                format={(number) => `${number}px at 1080p`}
                onChange={(font_size) => set({ font_size })}
              />
              <Field label="Colour">
                <input
                  type="color"
                  aria-label="Watermark colour"
                  value={value.color}
                  onChange={(event) => set({ color: event.target.value })}
                  className="h-9 w-12 cursor-pointer rounded-md border bg-background p-1"
                />
              </Field>
            </div>
          </>
        )}
        {value.type !== "none" && (
          <>
            <Slider
              label="Opacity"
              value={value.opacity}
              min={0.05}
              max={1}
              step={0.05}
              format={(number) => `${Math.round(number * 100)}%`}
              onChange={(opacity) => set({ opacity })}
            />
            <div className="grid grid-cols-[auto_1fr] items-end gap-4">
              <Field label="Position">
                <PositionPicker
                  value={value.position}
                  onChange={(position) => set({ position })}
                />
              </Field>
              <Slider
                label="Margin"
                value={value.margin}
                min={0}
                max={15}
                step={0.5}
                format={(number) => `${number}%`}
                onChange={(margin) => set({ margin })}
              />
            </div>
          </>
        )}
      </div>
      <div className="grid content-start gap-1.5">
        <span className="text-muted-foreground text-xs">Preview</span>
        <WatermarkPreview settings={value} />
        <p className="text-2xs text-muted-foreground leading-relaxed">
          Added to the finished video after rendering, so changing it never
          redraws the animation.
        </p>
      </div>
    </div>
  );
}
