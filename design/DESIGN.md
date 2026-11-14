# Manim Studio: Design (Phase 1)

Status: draft for review · 2026-11-12
Mockups: [`mockups/index.html`](mockups/index.html) (open in a browser and use the tabs to switch screens and themes)

Manim Studio is a native Rust app built on GPUI-CE and rendered by the
[manim-rust](https://github.com/ItsTatsuya/manim-rust) engine. Teachers describe a
lesson, upload notes, or paste a script, and get a narrated, captioned video. The
visual baseline is [Frame](https://github.com/66HEX/frame): graphite surfaces, one
neutral primary, soft inset edges, floating controls over a stage.

---

## 1. Principles

1. **The video is the hero.** Chrome is neutral graphite and steps back. The only
   saturated pixels are the teacher's content and one signal colour for things in
   progress.
2. **Plan before you spend.** Every AI flow runs *storyboard → approve → build*.
   The storyboard is cheap, and code is written only after approval. Token cost is
   shown before and after each turn. Efficiency is a product feature, not just an
   implementation detail.
3. **Teacher language, not developer language.** "Scenes", "Narration",
   "House style", "Practice question". Code is one tab away (`Code` mode), never in
   the way.
4. **Nothing blocks.** Rendering, downloads, updates and repairs run in the
   background and report progress in place: the scene card, the queue row, the
   title bar.
5. **Instant feel.** Target a 16 ms frame budget, cold start under 400 ms, and
   live preview of the selected scene from the engine without encoding an MP4.

## 2. Information architecture

```
Window
├─ Title bar      logo · mode switch · usage meter (hover: 5-hour + weekly, reset times) · Queue · Render
├─ Sidebar        search (Ctrl K) · New video · Projects › Videos · House styles · Renders · Settings
└─ Content, by mode
   ├─ Home        big composer (prompt + attachments + options) · 3 starts · recent videos
   ├─ Storyboard  beats (thumb, narration, visual direction, time) · build estimate · sources
   ├─ Studio      stage + floating transport · scene strip · right panel (Assistant | Narration | Assets)
   └─ Code        per-scene TS editor with diagnostics (for power users)
Overlays          onboarding (4 steps) · command palette · update toast · confirm dialogs
Full pages        Renders & queue · Settings
```

The video lifecycle drives the mode switch: **Storyboard** (draft → approved) →
**Studio** (built, previewing, editing) → rendered. A video is a folder of
`scenes/*.ts` + `narration.md` + `assets/`, so a single scene can be edited and
re-rendered without touching the rest.

## 3. Screens

| # | Screen | Purpose | Key decisions |
|---|---|---|---|
| 1 | Onboarding | 4 steps: Welcome → Connect assistant → House style → Sample lesson | Detects the Claude Code / Codex CLIs and shows plan and sign-in status. "Install & sign in" runs the official installer. Credentials are never stored. The right half plays a real engine render, so the product sells itself. Skippable. |
| 2 | Home | Start a video | One heading, one composer, recent videos below a hairline. The composer accepts a prompt, documents (PDF/DOCX/PPTX/MD) or a full script. Attachments are compact chips. Style, length and voice sit behind one summary button. Provider marks use the official Claude and OpenAI logos. |
| 3 | Studio | Watch and refine | The stage is a Frame-style inset well with floating tool rail, transport and zoom. The scene strip shows per-scene status dots. The Assistant shows compact activity rows (edited file, check passed, rendering %) and a per-turn token line. |
| 4 | Storyboard | Review the plan | Each beat has a number, thumbnail sketch, the narration line, visual direction and duration. The side panel shows estimated build tokens as a % of the plan window, output settings and sources. Revisions use the planning model. |
| 5 | Renders | Queue and history | Frame's queue table pattern. Status chips: Rendering (signal + progress), Waiting, Fixing (try n of 3), Done, Paused (reason). Footer shows the engine backend (GPU/CPU, encoder). |
| 6 | Settings | Assistants & usage first | Live plan usage per provider (5-hour and weekly bars). Separate planning and build models. Auto-switch provider when a plan runs low. Lean context. Repair count. |

States every screen must design for: empty, loading (skeleton tones, no
spinners on surfaces), error (inline chip + action), offline (assistant disabled,
rendering still works), and narrow window (below 1240 px the sidebar collapses to a 48 px icon rail,
the breadcrumb shows only the current item, and shortcut hints hide; below
900 px the right panel becomes a drawer). The breadcrumb never wraps: items
truncate with an ellipsis, the parent first.

## 4. Design tokens

All tokens live in one Rust module (`studio-ui/src/theme.rs`) as `Hsla`
constants grouped in a `Theme` struct, with `light()` and `dark()` constructors.
The mockup's CSS variables use the same names. Dark is the default theme, and the
app follows the OS unless overridden.

### Colour

| Token | Dark | Light | Use |
|---|---|---|---|
| `window` | `#111113` | `#ececee` | Window background, title bar |
| `sidebar` | `#141416` | `#e3e3e7` | Sidebar |
| `surface` | `#1b1b1e` | `#f7f7f8` | Panels |
| `surface_2` | `#222226` | `#efeff1` | Rows, inputs, activity items |
| `surface_3` | `#2c2c31` | `#e4e4e8` | Selected row, active segment track, user bubble |
| `raised` | `#29292d` | `#ffffff` | Floating controls, popovers, toasts, active segment |
| `stage` | `#0c0c0e` | `#dcdce0` | Video well |
| `line` | `#ffffff0b` | `#0303020f` | Hairlines, inset edges |
| `line_strong` | `#ffffff16` | `#0303021c` | Popover edges, inputs |
| `fg` | `#ededf0` | `#232327` | Primary text |
| `fg_2` | `#a5a5ad` | `#4f4f57` | Secondary text |
| `fg_3` | `#87878f` | `#6b6b74` | Tertiary, meta (≥ 4.5:1 on `surface`) |
| `primary` / `on_primary` | `#e8e8ec` / `#1a1a1d` | `#27272b` / `#f7f7f8` | The one filled button per view (Render, Continue, Approve) |
| `signal` | `#f0b35a` | `#c77d12` | In-progress only: rendering, writing, current scene |
| `ok` | `#83c89b` | `#24724c` | Done, signed in, check passed |
| `err` | `#ed8588` | `#b83842` | Failed, paused |

Rules: `signal` never decorates; it always means "working right now". Each
status colour has a `*_soft` tint (≈10% alpha) for chip fills. There is no
gradient anywhere in the chrome.

### Typography

- **UI:** Inter (variable, bundled, OFL) with `cv11 ss01 ss03`. Tabular numerals
  for every time, size, percentage and count.
- **Code / filenames / timecode in code:** JetBrains Mono.
- Lesson fonts (Plus Jakarta Sans, Caveat, …) belong to house styles and appear
  only inside the video.

| Role | Size / line | Weight | Tracking |
|---|---|---|---|
| `display` | 28 / 32 | 600 | −0.025em |
| `title` | 16 / 22 | 600 | −0.012em |
| `body` | 13 / 19 | 400 | 0 |
| `label` (buttons, rows) | 13 / 19 | 500 | 0 |
| `small` (meta, chips) | 12 / 17 | 400–500 | 0 |
| `caps` (section labels) | 11 / 14 | 600 | +0.04em, uppercase |

### Space, radius, elevation

- 4 px grid. Panel gutter 10 px. Panel padding 12–16 px. Row height 30 px.
  Control height 30 px (28 px compact, 38 px large).
- Radius: window 14 · panel 12 · composer 12–16 · control 8 · segment item 6 · chip 6 · thumb 5.
- Elevation is edges, not drop shadows: `inset 0 1px 0 white/4%` plus a
  `line` ring. Only floating layers (transport, popover, toast) get a real
  shadow (`0 16 40 -8 black/60%`).

### Motion

- 120 ms ease-out for hover and press. 180 ms for segment indicator and panel
  swaps. 240 ms spring for popovers and toasts (scale 0.98 → 1, opacity).
- Progress bars animate width linearly. A live dot pulses its 3 px halo, never
  the dot itself.
- Respect OS reduced-motion: all transitions become opacity-only at 0 ms.

## 5. Components (GPUI)

Reuse Frame's proven patterns (GPL-3.0, so we reimplement them rather than copy
code unless we adopt the GPL):

`Button` (primary / secondary / ghost / icon, with `Kbd` hint) · `SegmentedTabs`
with animated indicator · `Select` + `FloatingPopover` · `Tooltip` with grace
period · `Slider` · `Toggle` · `Checkbox` · `ListRow` (icon, label, meta, status
dot) · `Chip` (neutral/signal/ok/err) · `ProgressBar` · `Scrollbar` (overlay) ·
`Toast` · `Dialog` · `CommandPalette` · `Composer` (multiline input, attachment
chips, option row, send) · `ActivityRow` · `SceneCard` · `StageView` (engine
frame texture + floating rail/transport) · `UsageMeter`.

Every interactive component needs a visible focus ring (`ring`, 2 px offset), a
keyboard path, and an accessible name. Hit targets are at least 28 px.

## 6. Token efficiency is designed in, not bolted on

These decisions shape the UI and the agent harness together.

1. **Two-stage pipeline.** The storyboard uses the small planning model; build
   uses the code model. Approval is a hard gate.
2. **Scene-scoped edits.** Each scene is its own file. "@scene" mentions and the
   selected scene card set the edit scope, and the assistant receives only that
   file and the API outline.
3. **Compact API context.** Ship a generated, minimal `manim.d.ts` digest and a
   house-style recipe sheet instead of letting the agent explore. Cache them via
   provider prompt caching where the CLI supports it.
4. **Documents are summarised once** into `sources/*.md` (outline + key facts)
   and reused across turns and videos.
5. **Local checks before the model sees errors.** `manim-rs check` runs in-process
   (sub-second). Only the failing span plus a one-line diagnostic is sent back for
   repair, with a maximum of 3 tries.
6. **Render cache per scene** (hash of scene source + style + voice), so edits
   re-render one scene, never the whole video.
7. **Visible cost:** pre-build estimate, per-turn usage line, title-bar meter, and
   auto-switch to the other plan when one runs low.

## 7. Architecture sketch (for Phase 2)

```
crates/
  studio-app      GPUI-CE binary: windows, views, actions, keymap
  studio-ui       theme, primitives, components (no domain logic)
  studio-core     projects, videos, scenes, queue, settings (pure Rust, tested)
  studio-agent    provider adapters: Claude Code CLI + Codex CLI (stream-json),
                  session resume, cancellation, usage polling, prompt assembly
  studio-render   manim-rust integration: preview frames → GPUI texture, render jobs,
                  per-scene cache, narration (Kokoro via ONNX) + captions
  studio-docs     PDF/DOCX/PPTX/MD ingestion → source summaries
  studio-updater  signed manifest (minisign/ed25519), delta download, staged swap on restart
```

Updater: a signed JSON manifest on GitHub Releases with Stable and Beta channels.
Updates download in the background and apply on next launch or via "Restart now"
(see the toast in Studio). Rollback keeps the previous version.

## 8. Phases

1. **Design** (this doc + mockups): done.
2. **Foundation / UI shell:** done (2026-11-13). Workspace with GPUI-CE pinned, `studio-ui`
   theme and components, custom window chrome, sidebar, all six screens plus the Code
   view on mock data, light/dark/system themes, persisted preferences. Added 2026-11-14:
   command palette (Ctrl K), sidebar icon rail, empty states, keyboard focus with focus
   rings (Tab / Shift+Tab, Enter or Space to press), six app colour palettes. Still open:
   screen-reader labels (GPUI-CE has no accessibility API yet).
3. **Engine:** done (2026-11-13). `studio-render`: live stage preview from manim-rust's
   script host (dry run + record, then `frame_at`), thumbnails and real durations per scene,
   file watching, render queue with one engine process per scene (`--progress json`),
   per-scene cache, ffmpeg join, house styles as engine themes, per-video style and output,
   Code view on the real scripts with engine diagnostics, onboarding playing the sample live.
4. **Assistant:** done (2026-11-13). `studio-agent`: Claude Code (`-p --output-format
   stream-json`, own system prompt file, no MCP/user hooks) and Codex (`exec --json`,
   `AGENTS.md`) adapters; detection, sign-in and install through the official CLIs; real plan
   usage from rate-limit events; storyboard → approve → build; scene-scoped edits with fresh
   sessions under Lean context; engine check after every turn and bounded repairs; per-turn
   budget for edits. Videos, storyboards and chats persist in the video folders. Codex is
   implemented from its documented JSON format but untested (not installed on the dev machine).
5. **Content:** done (2026-11-13). Narration per scene with Kokoro-82M (`studio-render::kokoro`):
   ONNX Runtime on the CPU through `ort` (statically linked, telemetry off), phonemes from a
   Rust port of misaki's English lexicon G2P (gold/silver dictionaries, stress and suffix
   rules, numbers and maths symbols spelled out; espeak-ng as a separate program for unknown
   words when present, else spelled out; on Windows Settings fetches the official 1.52 MSI and
   unpacks it into data/voices/espeak-ng without installing it). A model (Standard: int8,
   92 MB, the default; or High: fp32, 326 MB; the same speed on the CPU, about 6x real time), 11 English voices and the US/UK
   dictionaries download from Settings (pinned revisions, SHA-256 checked). The system voice (SAPI / `say` / `espeak-ng`) is the fallback until then. Lines
   are cached per voice, timed into scenes (`NARRATION.seconds`), played during preview and
   muxed into the MP4; editable lines; captions as `.srt` or burned in; `studio-docs` reads
   PDF/DOCX/PPTX/MD/TXT into the plan; assets imported by picker or drop. Not done: no
   part-of-speech tagger (heteronyms use misaki's default reading).
6. **Ship:** in place (2026-11-13), not yet exercised on real releases. `studio-updater`
   (minisign-signed manifest, SHA-256-checked zip, staged swap on restart, rollback),
   enabled when built with `STUDIO_UPDATE_PUBKEY`; local crash reports with a pre-filled
   issue link (nothing sent); no telemetry; `script/bundle-windows.ps1` (portable zip +
   manifest, tested) and an Inno Setup script (untested: Inno not installed); GitHub Actions
   CI for Windows, macOS and Linux plus a tag-triggered release job. macOS/Linux builds and
   code signing are untested.

## 9. Open questions for review

- Accent: keep amber `signal` as the only colour, or let each house style tint the chrome?
- Should `Code` mode exist in v1 or come later behind a setting?
- Licensing: is GPL-3.0 acceptable (which would allow reusing Frame code directly), or do we stay permissive and reimplement?
- Platforms for v1: Windows only (like today) or Windows, macOS and Linux from day one?
