import type { AgentSettings } from "./api";
import type { SelectOption } from "../components/ui/select";

export const QUEUE_START_OPTIONS: SelectOption<AgentSettings["queue_start"]>[] =
  [
    {
      value: "auto",
      label: "Automatically",
      description: "Each video starts as soon as the one before it finishes.",
    },
    {
      value: "manual",
      label: "When I start the queue",
      description: "Videos wait in the queue until you press Start queue.",
    },
  ];

export const QUEUE_ERROR_OPTIONS: SelectOption<
  AgentSettings["queue_on_error"]
>[] = [
  {
    value: "fix",
    label: "Fix and re-render",
    description:
      "The agent repairs the script and the video rejoins the queue (up to 3 repairs).",
  },
  {
    value: "skip",
    label: "Skip it",
    description: "Mark the video as failed and carry on with the rest.",
  },
  {
    value: "pause",
    label: "Pause the queue",
    description: "Stop after the failure so you can look at it first.",
  },
];

export function videosLabel(count: number) {
  return `${count} ${count === 1 ? "video" : "videos"}`;
}
