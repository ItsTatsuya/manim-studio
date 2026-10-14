import { useState } from "react";
import { choosePreset, type OutputPreset } from "~/lib/render";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogBody,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";

export function OutputSettings({
  open,
  onOpenChange,
  value,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  value: OutputPreset;
}) {
  const [width, setWidth] = useState(String(value.width));
  const [height, setHeight] = useState(String(value.height));
  const [fps, setFps] = useState(String(value.fps));
  const [error, setError] = useState("");
  const save = () => {
    const dimensions = [Number(width), Number(height)];
    if (
      dimensions.some(
        (number) =>
          !Number.isInteger(number) ||
          number < 2 ||
          number > 32766 ||
          number % 2,
      )
    ) {
      setError("Width and height must be even pixel counts from 2 to 32766.");
      return;
    }
    const rate = Number(fps);
    if (!Number.isFinite(rate) || rate <= 0) {
      setError("Frame rate must be a positive number.");
      return;
    }
    choosePreset({
      id: "custom",
      label: "Custom",
      detail: "",
      quality: "standard",
      width: dimensions[0],
      height: dimensions[1],
      fps: rate,
    });
    onOpenChange(false);
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogPopup className="max-w-sm">
        <form
          onSubmit={(event) => {
            event.preventDefault();
            save();
          }}
        >
          <DialogHeader>
            <DialogTitle>Custom video output</DialogTitle>
          </DialogHeader>
          <DialogBody>
            <div className="grid grid-cols-2 gap-3">
              <label className="space-y-1 text-xs">
                Width
                <Input
                  aria-label="Width"
                  type="number"
                  min="2"
                  max="32766"
                  step="2"
                  value={width}
                  onChange={(event) => setWidth(event.target.value)}
                />
              </label>
              <label className="space-y-1 text-xs">
                Height
                <Input
                  aria-label="Height"
                  type="number"
                  min="2"
                  max="32766"
                  step="2"
                  value={height}
                  onChange={(event) => setHeight(event.target.value)}
                />
              </label>
            </div>
            <label className="mt-3 block space-y-1 text-xs">
              Frame rate
              <Input
                aria-label="Frame rate"
                type="number"
                step="any"
                value={fps}
                onChange={(event) => setFps(event.target.value)}
              />
            </label>
            {error && (
              <p
                role="alert"
                className="mt-3 text-xs text-destructive-foreground"
              >
                {error}
              </p>
            )}
          </DialogBody>
          <DialogFooter>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit">Use custom output</Button>
          </DialogFooter>
        </form>
      </DialogPopup>
    </Dialog>
  );
}
