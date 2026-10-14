import {
  CheckIcon,
  CopyIcon,
  DownloadIcon,
  LogInIcon,
  RefreshCwIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, type ProviderId, type ProviderStatus } from "~/lib/api";
import { useStudio } from "~/state/studio";
import { ProviderIcon, StudioMark } from "./icons";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
} from "./ui/dialog";
import { Badge, Spinner } from "./ui/input";
import { toast } from "./ui/toast";

const steps = ["Connect", "Quick guide", "Get started"];
const example =
  "Create a 15-second animation explaining the Pythagorean theorem with clear labels and calm voice narration. Generate the script and any assets, then check and render it.";
const guide = [
  [
    "Describe your animation",
    "Create a project, choose Codex or Claude, and describe the idea, duration, style and audience in chat.",
  ],
  [
    "Add your references",
    "Use Attach files to upload documents, images or audio. Dragging files into chat works too.",
  ],
  [
    "Review the result",
    "The agent writes your Manim script, narration and assets. Open Script or Narration to edit them, and Preview voice to hear the narration.",
  ],
  [
    "Render and refine",
    "Preview renders automatically after generation when enabled. Ask for changes in the same chat, or click Render yourself. Download the video from Preview or Renders.",
  ],
];

export function Onboarding({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const agents = useStudio((state) => state.agents);
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [selected, setSelected] = useState<ProviderId | null>(null);
  const title = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (open) {
      setStep(0);
      setSelected(null);
    }
  }, [open]);
  useEffect(() => {
    if (open) title.current?.focus();
  }, [open, step]);
  const refresh = async () => {
    setChecking(true);
    try {
      useStudio.setState({ agents: await api.refreshAgents() });
    } catch (error) {
      toast.error("Couldn't check providers", (error as Error).message);
    } finally {
      setChecking(false);
    }
  };
  const finish = async () => {
    setBusy(true);
    try {
      if (selected) {
        await api.saveAgentSettings({ default_provider: selected });
        await useStudio.getState().loadAgents();
      }
      await api.savePreferences({ "studio-onboarding": "2" });
      useStudio.setState((state) => ({
        prefs: { ...state.prefs, "studio-onboarding": "2" },
      }));
      onClose();
    } catch (error) {
      toast.error("Couldn't save setup", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const ready =
    agents?.providers.filter((provider) => provider.status === "ready") ?? [];
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !busy) void finish();
      }}
    >
      <DialogPopup
        className="max-h-[calc(100dvh-2rem)] max-w-2xl"
        showClose={false}
      >
        <DialogHeader className="shrink-0">
          <StudioMark className="mb-2 size-8 text-primary" />
          <DialogTitle ref={title} tabIndex={-1} className="pe-0 outline-none">
            {step === 0
              ? "Welcome to Manim Studio"
              : step === 1
                ? "From an idea to a video"
                : "Make your first animation"}
          </DialogTitle>
          <DialogDescription>
            {step === 0
              ? "Connect a coding assistant to generate scripts, narration and assets. You only need one."
              : step === 1
                ? "A quick tour of your workspace."
                : "Start with a small idea, then refine it in chat."}
          </DialogDescription>
          <ol aria-label="Setup progress" className="mt-3 flex gap-2">
            {steps.map((label, index) => (
              <li
                key={label}
                aria-current={step === index ? "step" : undefined}
                className={`flex min-w-0 flex-1 items-center gap-1.5 border-t-2 pt-2 text-xs ${index <= step ? "border-primary text-foreground" : "border-border text-muted-foreground"}`}
              >
                <span>
                  {index < step ? (
                    <CheckIcon className="size-3.5" />
                  ) : (
                    index + 1
                  )}
                </span>
                {label}
              </li>
            ))}
          </ol>
        </DialogHeader>
        <DialogBody className="space-y-4">
          {step === 0 && (
            <>
              <div className="flex items-center justify-between gap-3">
                <p className="text-xs text-muted-foreground" role="status">
                  {checking || agents?.probing || !agents
                    ? "Checking installed CLIs…"
                    : ready.length
                      ? `${ready.length} ${ready.length === 1 ? "assistant is" : "assistants are"} ready`
                      : "Install and log in to an assistant, or set one up later."}
                </p>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={checking || agents?.probing || !agents}
                  onClick={() => void refresh()}
                >
                  {checking || agents?.probing ? (
                    <Spinner />
                  ) : (
                    <RefreshCwIcon />
                  )}{" "}
                  Check again
                </Button>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                {agents ? (
                  agents.providers.map((provider) => (
                    <SetupProvider
                      key={provider.id}
                      provider={provider}
                      selected={selected === provider.id}
                      onSelect={() => setSelected(provider.id)}
                    />
                  ))
                ) : (
                  <Spinner />
                )}
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground">
                Installation and login open in a separate CLI window. Finish
                there, then choose Check again. Studio uses your existing
                provider account; its plan and usage limits apply.
              </p>
            </>
          )}
          {step === 1 && (
            <>
              <ol className="space-y-4">
                {guide.map(([heading, body], index) => (
                  <li key={heading} className="flex gap-3">
                    <span className="grid size-7 shrink-0 place-items-center rounded-full bg-primary/15 text-xs font-medium text-primary">
                      {index + 1}
                    </span>
                    <div>
                      <h2 className="text-sm font-medium">{heading}</h2>
                      <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                        {body}
                      </p>
                    </div>
                  </li>
                ))}
              </ol>
              <p className="rounded-lg border bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
                Voice models download the first time you use narration. Files
                and renders stay in your project. AI generation sends your
                prompt and any references the assistant reads to your chosen
                provider.
              </p>
            </>
          )}
          {step === 2 && (
            <>
              <div className="rounded-xl border bg-card p-4">
                <h2 className="text-sm font-medium">Try this prompt</h2>
                <p className="mt-2 text-sm leading-relaxed">{example}</p>
                <Button
                  className="mt-3"
                  size="sm"
                  variant="outline"
                  onClick={() =>
                    void navigator.clipboard
                      .writeText(example)
                      .then(() => toast.success("Example prompt copied"))
                      .catch(() => toast.error("Couldn't copy prompt"))
                  }
                >
                  <CopyIcon /> Copy example prompt
                </Button>
              </div>
              <p className="text-sm leading-relaxed text-muted-foreground">
                {ready.length
                  ? "Your assistant is ready. Paste the prompt into a project chat and press Enter."
                  : "You can edit and render Manim scripts manually now. Connect an assistant in Settings → Providers whenever you're ready."}
              </p>
              <p className="text-xs text-muted-foreground">
                You can reopen this setup and guide from Settings → General.
              </p>
            </>
          )}
        </DialogBody>
        <DialogFooter className="shrink-0 flex-wrap">
          <Button
            variant="ghost"
            className="me-auto"
            disabled={busy}
            onClick={() => void finish()}
          >
            Set up later
          </Button>
          {step > 0 && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => setStep(step - 1)}
            >
              Back
            </Button>
          )}
          <Button
            disabled={busy}
            onClick={() => (step < 2 ? setStep(step + 1) : void finish())}
          >
            {busy && <Spinner />}
            {step === 2 ? "Open Studio" : "Continue"}
          </Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}

function SetupProvider({
  provider,
  selected,
  onSelect,
}: {
  provider: ProviderStatus;
  selected: boolean;
  onSelect: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const action = async (kind: "install" | "login" | "enable") => {
    setBusy(true);
    try {
      if (kind === "enable") {
        await api.saveAgentSettings({
          providers: { [provider.id]: { enabled: true } },
        });
        await useStudio.getState().loadAgents();
      } else {
        if (kind === "install") await api.agentInstall(provider.id);
        else await api.agentLogin(provider.id);
        toast.success(
          kind === "install" ? "Installer opened" : "Login opened",
          "Complete it in the CLI window, then click Check again.",
        );
      }
    } catch (error) {
      toast.error(
        `Couldn't ${kind === "install" ? "open installer" : kind === "login" ? "open login" : "enable provider"}`,
        (error as Error).message,
      );
    } finally {
      setBusy(false);
    }
  };
  const checking = provider.status === "checking";
  const connected = provider.status === "ready";
  return (
    <section
      aria-label={`${provider.name} setup`}
      className={`min-w-0 rounded-xl border p-4 ${selected ? "border-primary bg-primary/5" : "bg-card"}`}
    >
      <div className="flex items-center gap-2">
        <ProviderIcon provider={provider.id} className="size-5" />
        <h2 className="text-sm font-medium">{provider.name}</h2>
      </div>
      <div className="mt-3">
        <Badge tone={connected ? "success" : "neutral"}>
          {checking
            ? "Checking"
            : !provider.enabled
              ? "Disabled"
              : !provider.installed
                ? "Not installed"
                : provider.auth.status === "unauthenticated"
                  ? "Login needed"
                  : connected
                    ? "Ready"
                    : "Needs attention"}
        </Badge>
      </div>
      <p
        className="mt-2 min-h-9 break-words text-xs leading-relaxed text-muted-foreground"
        role="status"
      >
        {provider.message ||
          (connected
            ? "Installed and logged in."
            : "Check the provider to see its status.")}
      </p>
      {provider.version && (
        <p className="mt-1 text-xs text-muted-foreground">{provider.version}</p>
      )}
      {!checking && !provider.installed && provider.enabled && (
        <div className="mt-3 space-y-2">
          <code className="block break-all rounded-md bg-code p-2 text-xs">
            {provider.install}
          </code>
          {provider.id === "codex" && (
            <p className="text-xs text-muted-foreground">
              Requires{" "}
              <a
                className="text-primary underline"
                href="https://nodejs.org/en/download"
                target="_blank"
                rel="noreferrer"
              >
                Node.js LTS
              </a>
              .
            </p>
          )}
        </div>
      )}
      <div className="mt-3 flex flex-wrap gap-2">
        {!provider.enabled ? (
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void action("enable")}
          >
            Enable
          </Button>
        ) : connected ? (
          <Button
            size="sm"
            variant={selected ? "secondary" : "outline"}
            aria-pressed={selected}
            onClick={onSelect}
          >
            {selected && <CheckIcon />}
            {selected ? "Default assistant" : `Use ${provider.name}`}
          </Button>
        ) : (
          <Button
            size="sm"
            disabled={checking || busy}
            onClick={() =>
              void action(provider.installed ? "login" : "install")
            }
          >
            {busy ? (
              <Spinner />
            ) : provider.installed ? (
              <LogInIcon />
            ) : (
              <DownloadIcon />
            )}
            {provider.installed ? "Log in" : "Install"}
          </Button>
        )}
        <a
          href={provider.docs}
          target="_blank"
          rel="noreferrer"
          className="self-center text-xs text-muted-foreground underline underline-offset-4"
        >
          Setup docs
        </a>
      </div>
    </section>
  );
}
