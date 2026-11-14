# Manim Studio 2

A native rewrite of [Manim Studio](https://github.com/ItsTatsuya/manim-studio) in Rust on
[GPUI-CE](https://github.com/gpui-ce/gpui-ce). Teachers describe a lesson, drop in notes or
paste a script, and get a narrated, captioned video rendered by
[manim-rust](https://github.com/ItsTatsuya/manim-rust).

**Status: Phases 1–6 built** (see [design/DESIGN.md §8](design/DESIGN.md)). Describe a
lesson (optionally with PDFs, Word or PowerPoint notes) and the assistant you are signed in
to (Claude Code or Codex) writes a storyboard; approve it and it writes one TypeScript
scene per file, which Studio checks with the engine and repairs. The stage previews scenes
live with narration, and renders produce a narrated, captioned MP4 with a per-scene cache.

## Run

```sh
cargo run -p studio-app            # debug
cargo run -p studio-app --release  # optimized
```

MP4 output needs ffmpeg (`winget install ffmpeg`); it is found on `PATH`, through
`MANIM_FFMPEG`, or in winget's package folder. Preview works without it. The assistant needs
the Claude Code or Codex CLI, signed in (Studio can install and start sign-in for you).

Release build: `script\bundle-windows.ps1` (portable zip + update manifest in `dist\`;
`-Installer` adds the Inno Setup installer). Updates are only enabled in builds made with
`STUDIO_UPDATE_PUBKEY` set to the release minisign public key.

First launch opens onboarding. Preferences are saved to the platform config directory
(`%APPDATA%\ManimStudio\Manim Studio\config\settings.json` on Windows). Delete that file to
see onboarding again, or use **Settings → General → Open setup**.

For screenshots and visual checks, open a screen directly:

```sh
STUDIO_SCREEN=studio cargo run -p studio-app   # onboarding|home|storyboard|studio|code|renders|settings
STUDIO_SCREEN=onboarding STUDIO_STEP=2 cargo run -p studio-app
```

## Layout

```
crates/
  studio-ui/     design system: theme tokens, embedded assets, components
    assets/      Inter + JetBrains Mono (OFL), stroke icons, provider marks, placeholder art
    src/theme.rs        colours, type scale, radius, spacing (mirrors DESIGN.md §4)
    src/components/     Button, Kbd, Chip, Dot, ProgressBar, Toggle, Segmented,
                        Dropdown/MenuItem, ListRow, TextInput (wrapping, IME, clipboard)
  studio-agent/  assistant adapters: Claude Code / Codex CLIs (stream JSON), detection,
                 sign-in, prompts (API digest, storyboard, build, edit, repair)
  studio-docs/   text out of PDF, DOCX, PPTX, Markdown and plain text
  studio-updater/ signed manifest (minisign), verified download, staged swap, rollback
  studio-render/ the engine layer on manim-rust (no UI dependency)
    src/preview.rs      live preview + thumbnails: one ScriptHost per thread, coalesced
                        seeks, reloads a scene when its files change
    src/queue.rs        render queue: one engine process per scene (`--progress json`),
                        cancel = kill, processes die with the app (job object on Windows)
    src/worker.rs       the engine process (`studio-engine` / `manim-studio render-worker`)
    src/cache.rs        scene cache key: engine rev + script + imports + assets + style + output
    src/export.rs       joins scene MP4s with ffmpeg (stream copy, x264 fallback)
    src/style.rs        house styles (Slate, Whiteboard, Clean, Phosphor, Playground, Documentary) as engine themes
    src/voice.rs        narration (Kokoro, else the system voice), WAV cache, captions
    src/kokoro/         Kokoro-82M on ONNX Runtime + misaki G2P port, model download
    samples/            the bundled "Area of a circle" lesson, installed on first run
  studio-app/    the desktop binary (+ `studio-engine`, the console-subsystem render worker)
    src/app.rs          Workspace root: navigation, actions, key bindings, playback
    src/engine.rs       stage preview, thumbnails, narration and queue wired in
    src/assistant.rs    storyboard → approve → build, scoped edits, checks and repairs
    src/store.rs        projects and videos on disk (video.json, chat.json)
    src/attach.rs       documents (Home) and assets (Studio) brought into a video
    src/audio.rs        narration playback during preview
    src/update.rs       background update checks, restart to update, rollback
    src/crash.rs        local crash reports and the pre-filled issue link
    src/model.rs        projects, scenes (with engine state), chat, render jobs
    src/prefs.rs        persisted preferences
    src/views/          chrome (title bar, window controls, sidebar), onboarding,
                        home, storyboard, studio (+ code view), renders, settings
packaging/windows/  Inno Setup installer script
script/             bundle-windows.ps1
.github/workflows/  CI (Windows, macOS, Linux) and the tagged release job
tools/
  icongen/       rasterizes brand/logo.svg into resources/icon.ico (`cargo run -p icongen`)
  tokenbench/    token benchmark for the assistant (see "Token benchmark" below)
design/
  DESIGN.md      design spec (principles, IA, tokens, components, phases)
  mockups/       HTML mockups the app is built from
```

## Token benchmark

`tools/tokenbench` builds fixed lessons with each prompt *recipe* and compares tokens with
quality. Lessons and variants (model, effort, recipe) live in `tools/tokenbench/bench.json`;
storyboards are planned once into `tools/tokenbench/plans/` so every variant and rerun builds
the same thing.

```sh
cargo run -p tokenbench --release -- --dry                         # prompt sizes, no assistant calls
cargo run -p tokenbench --release -- --cases fractions --variants baseline,example
cargo run -p tokenbench --release -- --runs 3                      # every case and variant, 3 times
```

Each run builds, checks every scene with the engine, repairs like the app does, and records
fresh input, cache writes, cache reads, output, model calls, the CLI's cost estimate and
time, against first-try and final pass rates, layout problems left, scene length vs plan
and code size. `target/tokenbench/<time>/summary.md` has the per-variant means;
`report.html` shows every scene's thumbnail per variant to judge the videos by eye. Real
runs use your assistant plan. The app uses `prompt::Recipe::default()`: change it there
when a variant wins on both tokens and quality.

## Keyboard

| Shortcut | Action |
|---|---|
| Ctrl/⌘ Enter | Render the current video |
| Ctrl/⌘ K | Search videos |
| Ctrl/⌘ N | New video |
| Ctrl/⌘ B | Toggle sidebar |
| Ctrl/⌘ , | Settings |
| Ctrl/⌘ R | Renders & queue |
| Enter / Shift Enter | Send / new line in a composer |
| Esc | Close menus |

## Notes

- GPUI-CE is pinned to the same revision Frame ships (`bc57747`). Bump it deliberately.
- GPUI draws SVG icons as alpha masks, so components always pass an explicit colour.
- manim-rust is pinned (`3deae09`, workspace `Cargo.toml`); `studio_render::ENGINE_REV` is
  part of every cache key, so bump both together.
- A video is a folder: `scenes/*.ts` (one scene per file, in order), `lib/`, `assets/`, plus
  `.cache/` and `exports/` written by the studio. The sample lives in
  `%APPDATA%\ManimStudio\Manim Studio\data\projects\`. Edit a scene in any editor and the
  stage reloads it.
- Scenes don't hard-code colours: the studio applies the house style as the engine theme, and
  scripts read `this.theme.colors` and use text roles.
- `cargo test -p studio-render` runs every sample scene in every house style through the
  engine's layout check. `cargo run -p studio-render --release --example snapshots -- <dir>`
  writes PNG stills for review.
- Assistant turns run the official CLI in the video folder with a short system prompt of our
  own (`AGENTS.md`: rules plus a comment-free API digest, identical every turn so it is
  prompt-cached), no MCP servers and no user hooks. Edits name one scene file; with Lean
  context each edit starts a fresh session. The engine checks every changed scene; failures
  go back as one diagnostic line plus the failing source line, up to the repair limit.
- Narration uses Kokoro-82M (Apache-2.0) with phonemes from a port of misaki (Apache-2.0);
  ONNX Runtime (MIT) is linked into the binary. The model downloads from Settings → Voice.
  espeak-ng (GPL) is only ever run as a separate program, for words not in the dictionaries;
  Settings → Voice can fetch it on Windows (unpacked from the official MSI, no install).
  Distributing a build needs these projects' licence notices.
- Watermarks (Settings → Rendering): text or an image, placed on a 3×3 grid with size,
  opacity, margin, colour and shadow, previewed live with the same geometry the export uses
  (`studio_render::export::place`). Drawn once on the exported MP4 in the same ffmpeg pass
  as burned-in captions, so scene caches stay valid. Projects can opt out (scene strip →
  Watermark; saved in `project.json`).
- Narration is in step with the picture sentence by sentence: each sentence is spoken and
  cached on its own, scenes call `this.narrate(n)` right before the animation sentence `n`
  describes (it returns the sentence's length and holds until the previous one ends), and
  the engine lays the sentences out at those marks for the preview, the render and the
  captions. Scenes without `narrate` play their sentences back to back and have their
  pauses shortened to fit. Scenes cross-dissolve into one continuous video.
- Headless render (no window): `cargo build -p studio-app --bin studio-engine`, then
  `cargo run -p studio-render --example render_video -- <video folder> [out.mp4]`.
- Debug builds keep line tables only (`[profile.dev]` in `Cargo.toml`); full debug info
  made `target/` grow past 60 GB.
- Repairs: after a turn, every failing scene goes back in one repair turn, each with its
  diagnostic and the source lines involved (for layout problems, the lines creating the
  overlapping texts).
- The app mark lives in `crates/studio-ui/assets/brand/logo.svg`; rerun `icongen` after editing it.
