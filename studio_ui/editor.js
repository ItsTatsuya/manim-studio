import { EditorState, Compartment } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  highlightActiveLineGutter,
  drawSelection,
  rectangularSelection,
  crosshairCursor,
  Decoration,
  ViewPlugin,
  placeholder,
} from "@codemirror/view";
import {
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import { python } from "@codemirror/lang-python";
import {
  indentUnit,
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  syntaxHighlighting,
  HighlightStyle,
} from "@codemirror/language";
import {
  autocompletion,
  completionKeymap,
  closeBrackets,
  closeBracketsKeymap,
} from "@codemirror/autocomplete";
import {
  search,
  searchKeymap,
  openSearchPanel,
  highlightSelectionMatches,
} from "@codemirror/search";
import { setDiagnostics, lintGutter } from "@codemirror/lint";
import { tags } from "@lezer/highlight";

const manimNames = {
  Scene: "Animation scene",
  MovingCameraScene: "Scene with a movable camera",
  ThreeDScene: "3D scene",
  Create: "Draw an object",
  Transform: "Transform one object into another",
  FadeIn: "Reveal an object",
  FadeOut: "Hide an object",
  Write: "Write text or equations",
  MathTex: "Mathematical typesetting",
  Tex: "LaTeX text",
  Text: "Ordinary text",
  Circle: "Circle shape",
  Square: "Square shape",
  Rectangle: "Rectangle shape",
  Line: "Straight line",
  Arrow: "Arrow shape",
  Dot: "Point marker",
  VGroup: "Group of vector objects",
  Axes: "Coordinate axes",
  NumberPlane: "Coordinate grid",
  ValueTracker: "Track an animated value",
  always_redraw: "Redraw on each frame",
  UP: "Up direction",
  DOWN: "Down direction",
  LEFT: "Left direction",
  RIGHT: "Right direction",
  ORIGIN: "Center of the scene",
  PI: "π",
  TAU: "2π",
  WHITE: "White color",
  BLUE: "Blue color",
  RED: "Red color",
  GREEN: "Green color",
  YELLOW: "Yellow color",
  config: "Manim configuration",
};
function completions(context) {
  const word = context.matchBefore(/\w*/);
  if (!context.explicit && !word.text) return null;
  return {
    from: word.from,
    options: Object.entries(manimNames).map(([label, detail]) => ({
      label,
      detail,
      type: label[0] === label[0].toUpperCase() ? "class" : "function",
    })),
    validFor: /^\w*$/,
  };
}
function indentGuides(view) {
  const ranges = [];
  for (const { from, to } of view.visibleRanges) {
    for (let position = from; position <= to;) {
      const line = view.state.doc.lineAt(position),
        indent = line.text.match(/^ +/)?.[0].length || 0;
      if (indent)
        ranges.push(
          Decoration.line({
            attributes: {
              class: "cm-indent-guides",
              style: `--indent-columns:${indent}`,
            },
          }).range(line.from),
        );
      if (line.to >= to) break;
      position = line.to + 1;
    }
  }
  return Decoration.set(ranges);
}
const guides = ViewPlugin.fromClass(
  class {
    constructor(view) {
      this.decorations = indentGuides(view);
    }
    update(update) {
      if (update.docChanged || update.viewportChanged)
        this.decorations = indentGuides(update.view);
    }
  },
  { decorations: (plugin) => plugin.decorations },
);
const colors = HighlightStyle.define([
  {
    tag: [tags.keyword, tags.modifier, tags.operatorKeyword],
    color: "var(--syntax-keyword)",
  },
  {
    tag: [tags.string, tags.special(tags.string)],
    color: "var(--syntax-string)",
  },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--syntax-number)" },
  { tag: tags.comment, color: "var(--quiet)" },
  {
    tag: [tags.className, tags.typeName, tags.definition(tags.variableName)],
    color: "var(--syntax-class)",
  },
  { tag: tags.function(tags.variableName), color: "var(--syntax-function)" },
]);
window.StudioEditor = {
  create(parent, { source = "", onChange, onSelection }) {
    const wrapping = new Compartment(),
      editing = new Compartment();
    let readOnly = false;
    const view = new EditorView({
      parent,
      state: EditorState.create({
        doc: source,
        extensions: [
          lineNumbers(),
          history(),
          drawSelection(),
          rectangularSelection(),
          crosshairCursor(),
          highlightActiveLine(),
          highlightActiveLineGutter(),
          highlightSelectionMatches(),
          bracketMatching(),
          closeBrackets(),
          foldGutter(),
          indentOnInput(),
          indentUnit.of("    "),
          python(),
          syntaxHighlighting(colors),
          lintGutter(),
          guides,
          search({ top: true }),
          autocompletion({ override: [completions] }),
          wrapping.of([]),
          editing.of([]),
          placeholder("# Paste your Manim script here"),
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...historyKeymap,
            ...searchKeymap,
            ...completionKeymap,
            ...foldKeymap,
            indentWithTab,
          ]),
          EditorState.tabSize.of(4),
          EditorView.contentAttributes.of({
            "aria-label": "Manim Python script",
            "aria-description":
              "Tab indents. Press Escape, then Tab to leave the editor. Ctrl+Space opens autocomplete.",
            spellcheck: "false",
            tabindex: "0",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) onChange(update.state.doc.toString());
            if (update.selectionSet || update.docChanged) {
              const head = update.state.selection.main.head,
                line = update.state.doc.lineAt(head);
              onSelection(line.number, head - line.from + 1);
            }
          }),
        ],
      }),
    });
    return {
      view,
      getSource: () => view.state.doc.toString(),
      setSource(source) {
        if (readOnly) return;
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: source },
          selection: { anchor: 0 },
        });
      },
      focus: () => view.focus(),
      find: () => openSearchPanel(view),
      setReadOnly(enabled) {
        if (readOnly === enabled) return;
        readOnly = enabled;
        view.dispatch({
          effects: editing.reconfigure([
            EditorState.readOnly.of(enabled),
            EditorView.editable.of(!enabled),
          ]),
        });
        view.contentDOM.setAttribute("aria-readonly", String(enabled));
      },
      wrap(enabled) {
        view.dispatch({
          effects: wrapping.reconfigure(enabled ? EditorView.lineWrapping : []),
        });
      },

      jump(line) {
        const target = view.state.doc.line(
          Math.max(1, Math.min(line, view.state.doc.lines)),
        );
        view.dispatch({
          selection: { anchor: target.from },
          effects: EditorView.scrollIntoView(target.from, { y: "center" }),
        });
        view.focus();
      },
      diagnostics(problems) {
        view.dispatch(
          setDiagnostics(
            view.state,
            problems
              .filter((problem) => problem.line)
              .map((problem) => {
                const line = view.state.doc.line(
                  Math.max(1, Math.min(problem.line, view.state.doc.lines)),
                );
                return {
                  from: line.from,
                  to: Math.max(line.from, line.to),
                  severity: problem.severity || "error",
                  message: problem.message,
                };
              }),
          ),
        );
        const invalid = problems.some(
          (problem) => problem.severity === "error",
        );
        view.contentDOM.setAttribute("aria-invalid", String(invalid));
        view.contentDOM.setAttribute(
          "aria-describedby",
          invalid ? "script-status problem-list" : "script-status",
        );
      },
      requestMeasure: () => view.requestMeasure(),
    };
  },
};
