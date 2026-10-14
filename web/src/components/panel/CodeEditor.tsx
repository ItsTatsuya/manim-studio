import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type CompletionContext,
} from "@codemirror/autocomplete";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import { python } from "@codemirror/lang-python";
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  HighlightStyle,
  indentOnInput,
  indentUnit,
  syntaxHighlighting,
} from "@codemirror/language";
import { lintGutter, setDiagnostics } from "@codemirror/lint";
import {
  highlightSelectionMatches,
  openSearchPanel,
  search,
  searchKeymap,
} from "@codemirror/search";
import { Compartment, EditorState } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";

const MANIM_NAMES: Record<string, string> = {
  Scene: "Animation scene",
  MovingCameraScene: "Scene with a movable camera",
  ThreeDScene: "3D scene",
  Create: "Draw an object",
  Transform: "Transform one object into another",
  ReplacementTransform: "Replace one object with another",
  FadeIn: "Reveal an object",
  FadeOut: "Hide an object",
  Write: "Write text or equations",
  Indicate: "Briefly highlight an object",
  MathTex: "LaTeX math",
  Tex: "LaTeX text",
  MathTypst: "Typst math",
  Text: "Ordinary text",
  MarkupText: "Text with markup",
  Circle: "Circle shape",
  Square: "Square shape",
  Rectangle: "Rectangle shape",
  RoundedRectangle: "Rounded rectangle",
  Polygon: "Polygon shape",
  Line: "Straight line",
  Arrow: "Arrow",
  Dot: "Point marker",
  Brace: "Curly brace",
  VGroup: "Group of vector objects",
  Group: "Group of objects",
  Axes: "Coordinate axes",
  NumberPlane: "Coordinate grid",
  NumberLine: "Number line",
  ValueTracker: "Animated value",
  ImageMobject: "Image from assets/",
  SVGMobject: "SVG from assets/",
  always_redraw: "Redraw every frame",
  UP: "Up",
  DOWN: "Down",
  LEFT: "Left",
  RIGHT: "Right",
  ORIGIN: "Center of the frame",
  PI: "π",
  TAU: "2π",
  WHITE: "White",
  BLUE: "Blue",
  RED: "Red",
  GREEN: "Green",
  YELLOW: "Yellow",
};

function completions(context: CompletionContext) {
  const word = context.matchBefore(/\w*/);
  if (!word || (!context.explicit && !word.text)) return null;
  return {
    from: word.from,
    options: Object.entries(MANIM_NAMES).map(([label, detail]) => ({
      label,
      detail,
      type:
        label[0] === label[0].toUpperCase() && /[a-z]/.test(label)
          ? "class"
          : "constant",
    })),
    validFor: /^\w*$/,
  };
}

// Syntax colors that read well on both themes, in the spirit of T3 Code's code blocks.
const highlight = HighlightStyle.define([
  {
    tag: [
      tags.keyword,
      tags.modifier,
      tags.operatorKeyword,
      tags.controlKeyword,
    ],
    color: "light-dark(#7c3aed, #c4a7ff)",
  },
  {
    tag: [tags.string, tags.special(tags.string)],
    color: "light-dark(#047857, #8bd5a8)",
  },
  {
    tag: [tags.number, tags.bool, tags.null],
    color: "light-dark(#b45309, #f5b971)",
  },
  {
    tag: tags.comment,
    color: "light-dark(#71717a, #737373)",
    fontStyle: "italic",
  },
  {
    tag: [tags.className, tags.typeName, tags.definition(tags.variableName)],
    color: "light-dark(#1d4ed8, #8fb4ff)",
  },
  {
    tag: tags.function(tags.variableName),
    color: "light-dark(#0e7490, #7dd3e6)",
  },
  { tag: tags.propertyName, color: "light-dark(#3f3f46, #d4d4d8)" },
]);

export interface CodeEditorHandle {
  jump(line: number): void;
  find(): void;
  focus(): void;
}

export interface Problem {
  line: number;
  message: string;
  severity?: "error" | "warning";
}

export const CodeEditor = forwardRef<
  CodeEditorHandle,
  {
    value: string;
    revision: number;
    readOnly?: boolean;
    onChange: (value: string) => void;
    problems: Problem[];
    onCursor?: (line: number, column: number) => void;
  }
>(function CodeEditor(
  { value, revision, readOnly = false, onChange, problems, onCursor },
  ref,
) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const callbacks = useRef({ onChange, onCursor });
  callbacks.current = { onChange, onCursor };
  const editing = useRef(new Compartment());

  useEffect(() => {
    view.current = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: value,
        extensions: [
          editing.current.of([
            EditorState.readOnly.of(readOnly),
            EditorView.editable.of(!readOnly),
          ]),
          lineNumbers(),
          history(),
          drawSelection(),
          highlightActiveLine(),
          highlightActiveLineGutter(),
          highlightSelectionMatches(),
          bracketMatching(),
          closeBrackets(),
          foldGutter(),
          indentOnInput(),
          indentUnit.of("    "),
          EditorState.tabSize.of(4),
          python(),
          syntaxHighlighting(highlight),
          lintGutter(),
          search({ top: true }),
          autocompletion({ override: [completions] }),
          placeholder("# Write a Manim scene, or ask the agent to create one"),
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            ...searchKeymap,
            ...completionKeymap,
            ...foldKeymap,
            indentWithTab,
          ]),
          EditorView.contentAttributes.of({
            "aria-label": "Manim Python script",
            "aria-description":
              "Tab indents. Press Escape, then Tab to leave the editor. Ctrl+Space opens autocomplete.",
            spellcheck: "false",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged)
              callbacks.current.onChange(update.state.doc.toString());
            if (update.selectionSet || update.docChanged) {
              const head = update.state.selection.main.head;
              const line = update.state.doc.lineAt(head);
              callbacks.current.onCursor?.(line.number, head - line.from + 1);
            }
          }),
        ],
      }),
    });
    view.current.scrollDOM.tabIndex = 0;
    view.current.scrollDOM.setAttribute("aria-label", "Script scroll area");
    return () => view.current?.destroy();
    // The editor owns its document; external replacements arrive via `revision`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    view.current?.dispatch({
      effects: editing.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    });
    view.current?.contentDOM.setAttribute("aria-readonly", String(readOnly));
  }, [readOnly]);

  useEffect(() => {
    const editor = view.current;
    if (!editor || editor.state.doc.toString() === value) return;
    editor.dispatch({
      changes: { from: 0, to: editor.state.doc.length, insert: value },
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision]);

  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const doc = editor.state.doc;
    editor.dispatch(
      setDiagnostics(
        editor.state,
        problems.map((problem) => {
          const line = doc.line(Math.max(1, Math.min(problem.line, doc.lines)));
          return {
            from: line.from,
            to: line.to,
            severity: problem.severity ?? "error",
            message: problem.message,
          };
        }),
      ),
    );
    editor.contentDOM.setAttribute(
      "aria-invalid",
      String(problems.some((problem) => problem.severity !== "warning")),
    );
  }, [problems, revision]);

  useImperativeHandle(ref, () => ({
    jump(line) {
      const editor = view.current;
      if (!editor) return;
      const target = editor.state.doc.line(
        Math.max(1, Math.min(line, editor.state.doc.lines)),
      );
      editor.dispatch({
        selection: { anchor: target.from },
        effects: EditorView.scrollIntoView(target.from, { y: "center" }),
      });
      editor.focus();
    },
    find: () => view.current && openSearchPanel(view.current),
    focus: () => view.current?.focus(),
  }));

  return <div ref={host} className="min-h-0 flex-1 overflow-hidden" />;
});
