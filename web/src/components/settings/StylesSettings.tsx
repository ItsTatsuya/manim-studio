import { CopyIcon, PencilIcon, PlusIcon, Trash2Icon } from "lucide-react";
import { useEffect, useState } from "react";

import { api, type FontList, type HouseStyle } from "~/lib/api";
import { useStyleLibrary, useStyles } from "~/lib/styles";
import { useStudio } from "~/state/studio";
import { StyleSwatch } from "../styles/StyleSwatch";
import { FontPicker } from "../ui/font-picker";
import { Button } from "../ui/button";
import {
  Dialog,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPopup,
  DialogTitle,
  confirmAction,
} from "../ui/dialog";
import { Field } from "../ui/field";
import { Badge, Input, Spinner, Textarea } from "../ui/input";
import { Select } from "../ui/select";
import { toast } from "../ui/toast";
import { SettingsRow, SettingsSection } from "./SettingsLayout";

const COLOR_LABELS: [keyof HouseStyle["colors"], string][] = [
  ["background", "Background"],
  ["text", "Text"],
  ["muted", "Muted text"],
  ["primary", "Primary"],
  ["secondary", "Secondary"],
  ["accent", "Accent / highlight"],
  ["positive", "Positive"],
  ["negative", "Negative"],
  ["key_fill", "Key-idea box"],
  ["example_fill", "Example box"],
];

const SIZE_LABELS: [keyof HouseStyle["sizes"], string][] = [
  ["title", "Title"],
  ["heading", "Heading"],
  ["body", "Body"],
  ["small", "Small"],
];

export function StylesSettings() {
  const library = useStyleLibrary();
  const error = useStyles((state) => state.error);
  const [editing, setEditing] = useState<HouseStyle | null>(null);
  const run = async (action: () => Promise<unknown>, failure: string) => {
    try {
      await action();
      await useStyles.getState().load();
    } catch (problem) {
      toast.error(failure, (problem as Error).message);
    }
  };
  if (!library)
    return error ? (
      <p role="alert" className="text-destructive-foreground text-sm">
        {error}
      </p>
    ) : (
      <Spinner />
    );
  const duplicate = (style: HouseStyle) =>
    run(async () => {
      const copy = await api.createStyle({}, style.id);
      setEditing(copy);
    }, "Couldn't duplicate the style");
  return (
    <>
      <SettingsSection
        title="House style"
        description="Every video uses one style: its colours, fonts, sizes, pacing and rules. The agent follows it and scripts import the matching studio_style.py helper."
      >
        <SettingsRow
          title="Default style"
          description="Used by projects that don't choose their own in the Preview panel."
        >
          <Select
            aria-label="Default style"
            value={library.default}
            onChange={(id) =>
              void run(
                () => api.setDefaultStyle(id),
                "Couldn't change the default style",
              )
            }
            options={library.styles.map((style) => ({
              value: style.id,
              label: style.name,
              group: style.builtin ? "Built-in" : "Your styles",
            }))}
          />
        </SettingsRow>
      </SettingsSection>

      <SettingsSection
        title="Styles"
        description="Built-in styles are read-only; duplicate one to make it your own."
        action={
          <Button
            size="xs"
            variant="outline"
            onClick={() =>
              void duplicate(
                library.styles.find((style) => style.id === library.default)!,
              )
            }
          >
            <PlusIcon /> New style
          </Button>
        }
      >
        {library.styles.map((style) => (
          <div
            key={style.id}
            className="flex flex-col gap-4 px-4 py-4 sm:flex-row sm:items-center"
          >
            <StyleSwatch style={style} className="sm:w-48 sm:shrink-0" />
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2">
                <h3 className="truncate font-medium text-sm">{style.name}</h3>
                {style.builtin && <Badge>Built-in</Badge>}
                {style.id === library.default && (
                  <Badge tone="info">Default</Badge>
                )}
              </div>
              {style.description && (
                <p className="mt-1 text-muted-foreground text-xs leading-relaxed">
                  {style.description}
                </p>
              )}
              <p className="mt-1 truncate text-muted-foreground text-xs">
                {style.fonts.heading || "Default font"} ·{" "}
                {style.fonts.body || "Default font"} · {style.sizes.body} pt
                body
              </p>
              <div className="mt-3 flex flex-wrap gap-1.5">
                {!style.builtin && (
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => setEditing(style)}
                  >
                    <PencilIcon /> Edit
                  </Button>
                )}
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() => void duplicate(style)}
                >
                  <CopyIcon /> Duplicate
                </Button>
                {!style.builtin && (
                  <Button
                    size="xs"
                    variant="ghost-muted"
                    aria-label={`Delete ${style.name}`}
                    onClick={async () => {
                      if (
                        await confirmAction({
                          title: `Delete “${style.name}”?`,
                          description:
                            "Projects using it switch to the default style the next time they render or chat.",
                          confirm: "Delete style",
                          destructive: true,
                        })
                      )
                        void run(
                          () => api.deleteStyle(style.id),
                          "Couldn't delete the style",
                        );
                    }}
                  >
                    <Trash2Icon /> Delete
                  </Button>
                )}
              </div>
            </div>
          </div>
        ))}
      </SettingsSection>
      <StyleEditor
        style={editing}
        onClose={() => setEditing(null)}
        onSaved={() => void useStyles.getState().load()}
      />
    </>
  );
}

function FontInput({
  label,
  value,
  fonts,
  onChange,
}: {
  label: string;
  value: string;
  fonts: FontList | null;
  onChange: (value: string) => void;
}) {
  return (
    <Field label={label}>
      <FontPicker
        aria-label={`${label} font`}
        value={value}
        fonts={fonts}
        onCommit={onChange}
        className="[&_input]:h-10 [&_input]:text-sm"
      />
    </Field>
  );
}

function NumberInput({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <Field label={label}>
      <Input
        aria-label={label}
        type="number"
        min={min}
        max={max}
        step={step}
        value={Number.isFinite(value) ? value : ""}
        onChange={(event) => onChange(event.target.valueAsNumber)}
      />
    </Field>
  );
}

function StyleEditor({
  style,
  onClose,
  onSaved,
}: {
  style: HouseStyle | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [draft, setDraft] = useState<HouseStyle | null>(style);
  const [busy, setBusy] = useState(false);
  const [fonts, setFonts] = useState<FontList | null>(null);
  const projects = useStudio((state) => state.projects);
  const lastProject = useStudio((state) => state.prefs["manim-project"]);
  const [fontProject, setFontProject] = useState("");
  const open = !!style;
  // Start from the saved style each time the editor opens; keep the last draft
  // while the dialog animates out.
  useEffect(() => {
    if (style) setDraft(style);
  }, [style]);
  useEffect(() => {
    if (open && !fontProject)
      setFontProject(
        projects.some((project) => project.id === lastProject)
          ? lastProject
          : (projects[0]?.id ?? ""),
      );
  }, [open, fontProject, projects, lastProject]);
  useEffect(() => {
    if (!open) return;
    let current = true;
    api
      .fonts(fontProject || null)
      .then((list) => current && setFonts(list))
      .catch(() => current && setFonts({ system: [], project: [] }));
    return () => {
      current = false;
    };
  }, [open, fontProject]);
  const edit = <K extends keyof HouseStyle>(
    key: K,
    changes: Partial<HouseStyle[K]> | HouseStyle[K],
  ) =>
    setDraft((current) =>
      current
        ? {
            ...current,
            [key]:
              typeof changes === "object" && changes !== null
                ? { ...(current[key] as object), ...changes }
                : changes,
          }
        : current,
    );
  const save = async () => {
    if (!draft) return;
    setBusy(true);
    try {
      const { id: _id, builtin: _builtin, ...values } = draft;
      await api.updateStyle(draft.id, values);
      onSaved();
      onClose();
    } catch (error) {
      toast.error("Couldn't save the style", (error as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        if (!value && !busy) onClose();
      }}
    >
      <DialogPopup className="max-h-[calc(100dvh-2rem)] max-w-3xl">
        <form
          className="flex min-h-0 flex-1 flex-col"
          onSubmit={(event) => {
            event.preventDefault();
            void save();
          }}
        >
          <DialogHeader>
            <DialogTitle>Edit style</DialogTitle>
            <DialogDescription>
              Changes apply the next time a project using this style renders or
              chats.
            </DialogDescription>
          </DialogHeader>
          {draft && (
            <DialogBody className="space-y-6">
              <div className="sticky top-0 z-10 -mx-1 bg-popover px-1 pb-2">
                <StyleSwatch style={draft} className="mx-auto max-w-xs" />
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                <Field label="Name">
                  <Input
                    aria-label="Style name"
                    value={draft.name}
                    maxLength={60}
                    required
                    onChange={(event) => edit("name", event.target.value)}
                  />
                </Field>
                <Field label="Description">
                  <Input
                    aria-label="Style description"
                    value={draft.description}
                    maxLength={200}
                    onChange={(event) =>
                      edit("description", event.target.value)
                    }
                  />
                </Field>
              </div>

              <fieldset>
                <legend className="mb-2 font-medium text-sm">Colours</legend>
                <div className="grid grid-cols-2 gap-3 sm:grid-cols-5">
                  {COLOR_LABELS.map(([key, label]) => (
                    <Field key={key} label={label}>
                      <span className="flex items-center gap-1.5">
                        <input
                          type="color"
                          aria-label={`${label} colour`}
                          value={draft.colors[key].toLowerCase()}
                          onChange={(event) =>
                            edit("colors", {
                              [key]: event.target.value.toUpperCase(),
                            })
                          }
                          className="h-8 w-9 shrink-0 cursor-pointer rounded-md border bg-transparent p-0.5"
                        />
                        <span className="font-mono text-2xs text-foreground/80">
                          {draft.colors[key]}
                        </span>
                      </span>
                    </Field>
                  ))}
                </div>
              </fieldset>

              <fieldset>
                <legend className="mb-2 font-medium text-sm">Fonts</legend>
                <div className="grid gap-3 sm:grid-cols-2">
                  <FontInput
                    label="Headings"
                    fonts={fonts}
                    value={draft.fonts.heading}
                    onChange={(heading) => edit("fonts", { heading })}
                  />
                  <FontInput
                    label="Body"
                    fonts={fonts}
                    value={draft.fonts.body}
                    onChange={(body) => edit("fonts", { body })}
                  />
                  <FontInput
                    label="Code"
                    fonts={fonts}
                    value={draft.fonts.code}
                    onChange={(code) => edit("fonts", { code })}
                  />
                  <Field label="Equations">
                    <Select
                      aria-label="Equations"
                      value={draft.fonts.math}
                      onChange={(math) => edit("fonts", { math })}
                      options={[
                        { value: "auto", label: "Studio's choice" },
                        { value: "typst", label: "MathTypst" },
                        { value: "tex", label: "MathTex (LaTeX)" },
                      ]}
                    />
                  </Field>
                </div>
                <div className="mt-3 flex flex-wrap items-center gap-2 text-muted-foreground text-xs">
                  <span>
                    {fonts
                      ? `${fonts.bundled?.length ?? 0} Studio fonts and ${fonts.system.length} installed fonts`
                      : "Loading fonts…"}
                    {fonts?.project.length
                      ? `, plus ${fonts.project.length} from`
                      : ". Add .ttf or .otf files to a project's Assets to use them; fonts from"}
                  </span>
                  <Select
                    aria-label="Fonts from project"
                    size="xs"
                    value={fontProject}
                    placeholder="No projects"
                    onChange={setFontProject}
                    options={projects.map((project) => ({
                      value: project.id,
                      label: project.name,
                    }))}
                  />
                </div>
              </fieldset>

              <fieldset>
                <legend className="mb-2 font-medium text-sm">
                  Type scale and layout
                </legend>
                <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
                  {SIZE_LABELS.map(([key, label]) => (
                    <NumberInput
                      key={key}
                      label={label}
                      value={draft.sizes[key]}
                      min={8}
                      max={120}
                      onChange={(value) => edit("sizes", { [key]: value })}
                    />
                  ))}
                  <NumberInput
                    label="Safe margin"
                    value={draft.layout.margin}
                    min={0.1}
                    max={2}
                    step={0.05}
                    onChange={(margin) => edit("layout", { margin })}
                  />
                  <Field label="Titles">
                    <Select
                      aria-label="Title placement"
                      value={draft.layout.title}
                      onChange={(title) => edit("layout", { title })}
                      options={[
                        { value: "top-left", label: "Top left" },
                        { value: "top-center", label: "Top centre" },
                        { value: "center", label: "Centred" },
                      ]}
                    />
                  </Field>
                </div>
              </fieldset>

              <fieldset>
                <legend className="mb-2 font-medium text-sm">
                  Pacing and voice
                </legend>
                <div className="grid grid-cols-3 gap-3">
                  <NumberInput
                    label="Animation (s)"
                    value={draft.pacing.run_time}
                    min={0.2}
                    max={6}
                    step={0.1}
                    onChange={(run_time) => edit("pacing", { run_time })}
                  />
                  <NumberInput
                    label="Pause (s)"
                    value={draft.pacing.wait}
                    min={0}
                    max={6}
                    step={0.1}
                    onChange={(wait) => edit("pacing", { wait })}
                  />
                  <NumberInput
                    label="Words per second"
                    value={draft.pacing.words_per_second}
                    min={1}
                    max={4}
                    step={0.1}
                    onChange={(words_per_second) =>
                      edit("pacing", { words_per_second })
                    }
                  />
                </div>
                <Field label="Narration tone" className="mt-3">
                  <Input
                    aria-label="Narration tone"
                    value={draft.voice}
                    maxLength={200}
                    placeholder="e.g. Warm, encouraging teacher"
                    onChange={(event) => edit("voice", event.target.value)}
                  />
                </Field>
              </fieldset>

              <Field label="House rules">
                <Textarea
                  aria-label="House rules"
                  value={draft.rules}
                  maxLength={2000}
                  placeholder="e.g. Show learning objectives at the start. Always end with a recap slide."
                  onChange={(event) => edit("rules", event.target.value)}
                />
              </Field>
            </DialogBody>
          )}
          <DialogFooter>
            <Button variant="ghost" disabled={busy} onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || !draft}>
              Save style
            </Button>
          </DialogFooter>
        </form>
      </DialogPopup>
    </Dialog>
  );
}
