import {
  CodeIcon,
  FileTextIcon,
  ImageIcon,
  MicIcon,
  PlayCircleIcon,
} from "lucide-react";
import type * as React from "react";

import { useProjectDoc } from "~/state/project";
import { AssetsPanel } from "./AssetsPanel";
import { NarrationPanel } from "./NarrationPanel";
import { OutputPanel } from "./OutputPanel";
import { PreviewPanel } from "./PreviewPanel";
import { ScriptPanel } from "./ScriptPanel";
import { Button } from "../ui/button";

export type PanelTab = "preview" | "script" | "narration" | "assets" | "output";

const TABS: { id: PanelTab; label: string; icon: React.ReactNode }[] = [
  { id: "preview", label: "Preview", icon: <PlayCircleIcon /> },
  { id: "script", label: "Script", icon: <CodeIcon /> },
  { id: "narration", label: "Narration", icon: <MicIcon /> },
  { id: "assets", label: "Assets", icon: <ImageIcon /> },
  { id: "output", label: "Log", icon: <FileTextIcon /> },
];

export function RightPanel({
  projectId,
  tab,
  onTabChange,
  onRender,
}: {
  projectId: string;
  tab: PanelTab;
  onTabChange: (tab: PanelTab) => void;
  onRender: () => void;
}) {
  const analysis = useProjectDoc((state) => state.analysis);
  const narrationOn = useProjectDoc((state) => state.narration?.enabled);
  const saveStatus = useProjectDoc((state) => state.saveStatus);
  const saveError = useProjectDoc((state) => state.saveError);
  return (
    <>
      <div className="@container flex h-10 shrink-0 items-center border-b px-2">
        <div
          role="tablist"
          aria-label="Project panel"
          className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto px-0.5 py-1"
        >
          {TABS.map((item) => (
            <button
              key={item.id}
              role="tab"
              type="button"
              id={`tab-${item.id}`}
              aria-selected={tab === item.id}
              aria-controls={tab === item.id ? `panel-${item.id}` : undefined}
              tabIndex={tab === item.id ? 0 : -1}
              onClick={() => onTabChange(item.id)}
              onKeyDown={(event) => {
                const index = TABS.findIndex((entry) => entry.id === item.id);
                const next =
                  event.key === "ArrowRight"
                    ? (index + 1) % TABS.length
                    : event.key === "ArrowLeft"
                      ? (index + TABS.length - 1) % TABS.length
                      : event.key === "Home"
                        ? 0
                        : event.key === "End"
                          ? TABS.length - 1
                          : -1;
                if (next < 0) return;
                event.preventDefault();
                onTabChange(TABS[next].id);
                document.getElementById(`tab-${TABS[next].id}`)?.focus();
              }}
              className="studio-tab relative flex h-8 shrink-0 items-center gap-1.5 rounded-md px-2 text-ui text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&_svg]:size-3.5"
            >
              {item.icon}
              <span className="@max-[30rem]:sr-only">{item.label}</span>
              {item.id === "script" && analysis?.error && (
                <span
                  className="size-1.5 rounded-full bg-destructive"
                  aria-hidden="true"
                />
              )}
              {item.id === "narration" && narrationOn && (
                <span
                  className="size-1.5 rounded-full bg-success"
                  aria-hidden="true"
                />
              )}
            </button>
          ))}
        </div>
        <span
          className="ms-auto pe-1 text-2xs text-muted-foreground"
          aria-live="polite"
        >
          {saveStatus === "saving" || saveStatus === "pending"
            ? "Saving…"
            : saveStatus === "error"
              ? "Not saved"
              : ""}
        </span>
      </div>
      {saveStatus === "error" && (
        <div
          role="alert"
          className="flex items-center gap-2 border-b bg-destructive/5 p-3 text-xs"
        >
          <span className="min-w-0 flex-1 text-destructive-foreground">
            {saveError || "Your changes haven't been saved."}
          </span>
          <Button
            size="xs"
            variant="outline"
            onClick={() => void useProjectDoc.getState().flush()}
          >
            Retry save
          </Button>
        </div>
      )}
      <div
        role="tabpanel"
        id={`panel-${tab}`}
        aria-labelledby={`tab-${tab}`}
        className="flex min-h-0 flex-1 flex-col"
      >
        {tab === "preview" && (
          <PreviewPanel
            projectId={projectId}
            onRender={onRender}
            onShowTab={onTabChange}
          />
        )}
        {tab === "script" && <ScriptPanel projectId={projectId} />}
        {tab === "narration" && <NarrationPanel />}
        {tab === "assets" && <AssetsPanel projectId={projectId} />}
        {tab === "output" && <OutputPanel projectId={projectId} />}
      </div>
    </>
  );
}
