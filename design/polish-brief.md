# Manim Studio: polish brief

Put this file in the repo at `design/polish-brief.md` and work through it in order. Each
workstream ends with something you can check. The reference bundle (`reference/`) holds a
bake-off: scenes S03–S04 of the lesson "Two Equations, Two Lines: What Does Solving Mean"
(script `Video_1.01 … Script.docx`), built outside Studio. Its MP4s are the visual bar.
Its `spec.json` shows the timing model, and `Scene.tsx` / `index.html` show the same scene
as plain code.

## Why

Teacher review of a Studio render of this lesson found:

- The video ran ahead of the audio four times (4:11, 5:00, 5:37, and an equation that
  appeared before it was spoken).
- Two lines that should coincide were not drawn exactly on top of each other.
- Words the narrator stresses ("Gauss elimination", "parameter", the question "How many
  ways can two lines sit in a plane?") never appeared on screen.
- Substitutions were spoken but not shown as worked steps beside the graph.

The bake-off fixed all of these with the same voice (Kokoro) and similar visuals. The fixes
came from timing, layout and components, not from the renderer, so they can be built into
Studio and manim-rust.

Research behind the order of the work: in Code2Video (NeurIPS 2025, Claude writing Manim),
a planner plus a visual critic took aesthetics from 38 to 88 out of 100, against 99.7 for
3Blue1Brown. Removing the critic cost 21 points on the learning quiz, and removing its
fixed placement grid cost 27.

## Keep as they are

Storyboard → approve → build, the per-scene cache, the house styles, Kokoro and the
caption output, the engine layout check with the repair loop, `tokenbench`, and the
watermark settings. Everything below adds to these.

---

## 1. Word-level cues (fixes the sync complaints)

Today `this.narrate(n)` places whole sentences (`preview.rs`). Anything the voice names
in the middle of a sentence ("If r is ten, **c is twenty**") appears at the start of the
sentence, so the picture runs ahead of the voice.

Build:

- **Cue marks in narration text.** The storyboard and teacher scripts can mark moments in
  a sentence: `If r is ten, {c20}c is twenty.` The braces are removed before synthesis and
  in captions.
- **Cue times.** For each sentence, estimate every word's start from the phoneme count of
  each word: you already have G2P in `kokoro/g2p.rs`. Spread the sentence's measured
  length across its phonemes and add a fixed pause at each comma or semicolon. Then
  correct the estimate against the audio: snap each comma pause to the nearest stretch of
  silence in the WAV (RMS below a threshold for at least 80 ms). The current Kokoro ONNX
  model returns audio only, with no durations, so estimation is the practical route.
  Re-exporting the model with its duration output is a later upgrade.
- **Scene API.** `this.cue("c20")` waits until that cue's time, as `narrate` does for
  sentences, and returns the seconds left until the next cue or the end of the sentence.
  `narrate(n)` keeps working. Pass the cue times in `NARRATION` (`narration_json`) and
  cache them with the sentence WAVs.
- **Planner prompt (`prompt::plan`).** Ask for a cue at each thing that appears on screen
  in the middle of a sentence: a dot, a number, a named term.
- **Builder prompt (`build_system`).** Say: "Call `this.cue(id)` immediately before the
  animation for that moment. Never show a value before the voice says it."

Check: in the rebuilt S03, each dot pops within 0.25 s of its number being spoken. Measure
this from the cue marks in the scene's timeline, not by eye.

## 2. Layout regions instead of coordinates

The builder prompt places text with `nextTo` and coordinates, and the overlap rules are
written as prose. Replace free placement with named regions that the engine and kit
enforce. Frame: 14.2 × 8 units, about 135 px per unit at 1080p.

| Region | Use | Units (x, y) |
|---|---|---|
| `kicker` | small label above the work column | x −6.2…−1.3, y ≈ 3.0 |
| `work` | equations, worked steps, answer box | x −6.2…−1.3, y 2.8…−3.4 |
| `figure` | graph / diagram panel (rounded card) | x −0.45…6.5, y 3.2…−3.3 |
| `full` | full-width statement cards over a dimmed stage | centre, width ≤ 10 |
| `wm` | watermark, top right | from the Watermark settings |

- Add `lib/layout.ts` to the kit, or engine support if it's cleaner. Regions with a vertical
  stack: `work.push(mobject)` places the next item below the last with a fixed gap, and
  `work.clear()` fades the stack out.
- The layout check knows the regions: anything outside its region is a failure the repair
  loop sees.
- Prompt: "Place content only through regions. Never pass coordinates for text."

Check: rebuild S03–S07 with zero layout failures after repair, and no `vec(...)` for text
in the generated scenes.

## 3. A component kit for lessons

Polish comes from a few well-designed pieces used every time. Build them once in
`lib/kit.ts` (or as engine mobjects), each with designed enter, exit and emphasis motion,
and list them in the API digest:

- `EquationCard(tex, colour, tag)`: rounded panel with a colour bar and a tag ("ORDER 1"),
  plus `focus()` and `dim()` states.
- `Steps([...tex])`: numbered worked steps that enter one at a time (`next()`).
- `AnswerBox(tex, sub)`: green rounded box. Use the script's colour for final answers.
- `Callout(text, sub)`: a one-line idea such as "One equation = one line of answers."
- `Plot({xRange, yRange, step, xLabel, yLabel})` with:
  - `line(f, colour)`, drawn from end to end; `line(..., {dashed: true})` for a line that
    lands on another.
  - `dot([x, y], colour, label)`, which pops with overshoot.
  - `crossing([x, y])`: gold dot with two pulses, dashed guides to both axes, and gold axis
    labels that replace the tick numbers under them.
  - `slopeTriangle(...)`.
  - `probe(path)`: a ring that slides along a line with a live readout.
- `StatementCard(text, accentWord)`: full-region card over a dimmed, slightly blurred stage.
- `StrikeOut(mobject)` for mistakes, `PauseCard(seconds)`, `CaseTable(rows)`.

Lines that should coincide (S07 part b) must share the exact same geometry, with the second
drawn dashed on top. `Plot.line` should reuse the existing line's points when the function
is the same.

Check: the generated S03–S04 use only kit pieces for content, and the scene files get
shorter.

## 4. Engine work in manim-rust

Add these to `design/engine-issues.md` and do them in the engine:

1. **Camera.** `MovingCameraScene`-style frame: animate the frame's centre and width
   (push in on the crossing at (10, 20), then pull back). It's listed as missing in the
   engine README.
2. **Card effects.** Rounded rectangles with soft drop shadows; linear gradient fills on
   cards; background blur for the area behind a `StatementCard`. The CPU and GPU paths
   should match.
3. **Dash phase.** Animate the dash offset, so dashed guide lines draw on.

Check: the camera push in S04 matches the reference MP4 (scale about 1.22, eased in and out
over 2.2 s).

## 5. Render-and-look critic (after the engine check passes)

The engine check catches errors and overlaps. Add timing and readability checks, then a
look pass:

- **Dead air.** Nothing moves for more than 3 s while the voice runs → finding (unless
  marked `hold`).
- **Too brief.** A text block on screen for less than 0.5 s + 0.3 s per word → finding.
- **Too small.** Reading text under 28 px at 1080p → finding (the watermark is exempt).
- **Off-cue.** A value appears more than 0.4 s before its cue → finding.
- **Look pass (optional, costs tokens).** Render a still at the end of each cue, tile the
  stills into a contact sheet, and ask the assistant: "List overlaps, clipped content,
  empty halves of the screen, unreadable text, and anything on screen that the narration
  hasn't said yet." Send what it finds back through `repair_all`, with the frame time and
  the source line.

Every finding goes through the existing repair loop and counts against the repair limit.
Report the counts in `tokenbench`.

## 6. Scripts teachers already write

This script format is common: lesson plan, a developer brief (scenes, colours, axes, TeX),
then narration word for word with an on-screen action column, then reviewer notes.

- When a script like this is attached, the storyboard must use its narration **verbatim**
  and its colours and axis ranges as given. The planner adds cue marks only, never changes
  the wording.
- Read the brand table (logo, watermark size and position, end card) into the video's
  watermark settings and end scene.
- The script gives equations in LaTeX, but the engine uses Typst syntax. Convert them, and
  check each converted equation renders before building.
- Apply reviewer notes only where the teacher asks for them.

## 7. Audio (later)

- Optional voice: Chatterbox (MIT) beats ElevenLabs in a vendor blind test and supports
  cloning, so a school can use its own teacher's voice. Keep Kokoro as the default.
- A quiet music bed that dips under the voice, and soft sounds on dot pops and the answer
  box, each with a setting.

---

## Acceptance test

Rebuild S03–S04 of the attached script (1:15–3:00) in Studio and compare it with
`reference/S03-S04 HyperFrames.mp4`:

- [ ] Narration word for word, with reviewer notes 2–4 applied (they're also in the reference).
- [ ] Worked substitutions in the `work` region, each step appearing as it is spoken.
- [ ] Dots pop on their numbers (within 0.25 s); lines draw through the dots.
- [ ] Probe along the teal line, then off it (3r + 2c = 126 ✗).
- [ ] Camera push-in on (10, 20), gold pulse twice, guides, and gold axis labels replacing
      the ticks.
- [ ] Green answer box; then a full-width "Solving = finding the points common to all the
      lines." card over a dimmed stage.
- [ ] Zero findings from steps 2 and 5 after repair.
- [ ] Teal #2EC4B6, coral #FF6B6B, gold #F5B700, green #57CC99, as in the script brief.

Then run the whole lesson (S01–S10) and record first-try and final pass rates, tokens and
render time in `tokenbench` against the current recipe.

## Order and scope

Do 1 → 2 → 3 first: they fix what the teachers complained about. 4 is engine work that can
run alongside them. 5 builds on 2. 6 and 7 come last. Keep each workstream in its own
branch and pull request, with tests in the style of the existing ones (for example
`narrated_scenes_end_with_their_voice`).
