# Manim Studio project contract

This repository contains the standalone Windows GUI application. End users
install it or extract the portable ZIP and open Manim Studio.exe. The original
starter scripts and manual rendering launchers have been retired.

- Keep Manim Community Edition 0.21.0 and Python 3.12; do not use ManimGL or manimlib.
- Preserve the local FastAPI/Uvicorn, pywebview/WebView2 and CodeMirror architecture.
- The React 19 frontend in `web/` uses Vite, Tailwind 4, TanStack Router,
  Base UI and Zustand, following T3 Code's workspace architecture. FastAPI serves
  the compiled `studio_ui/`; edit `web/`, never the generated files. Keep T3 Code
  attribution and bundled npm licenses. `npm run dev` rebuilds on source changes.
- Agent sessions live in `DATA_ROOT/.studio/threads/<project-id>/`; provider
  settings live in `.studio/agents.json`. Use installed CLI authentication, never
  copy credentials. Keep structured streaming, session resume and process-tree
  cancellation. `full` bypasses CLI permissions; `edits` uses Codex's workspace
  sandbox or Claude's acceptEdits mode (Claude's mode is not an OS sandbox).
- Use the Cairo renderer unless a requested feature requires a deliberate change.
- Keep rendering in the worker process, with progress, cancellation and diagnostics.
- Keep app code separate from user data. Preserve scripts, preferences, assets and
  render history during updates; never commit them or generated videos.
- Keep named output folders and compatibility with existing GUI render manifests.
- Projects live in `DATA_ROOT/projects/<name>/` (`project.json`, `script.py`,
  `narration.txt`, `assets/`; see `projects.py`). Project renders run with that folder as the working
  directory so `assets/...` paths resolve there. Requests without a project keep
  the legacy data-root behavior.
- Folder and ZIP imports use `/api/projects/import`, preserve nested assets and
  create a new project transactionally. Reject traversal, duplicate Windows paths,
  ZIP symlinks and packages over 512 MB / 2000 files; never install package requirements.
- Narration (`narration.py`) uses Kokoro-82M through `kokoro-onnx` and ONNX Runtime
  on the CPU; do not add PyTorch. Model files download on demand into
  `DATA_ROOT/.studio/models/kokoro` from pinned release URLs with SHA256 checks.
  The render worker synthesizes and muxes narration in the same cancellable process.
  Narration is synthesized before Manim runs so script problems fail fast. Finished
  lines (`.studio/render-cache/lines`) and the silent animation (`render-cache/videos`,
  keyed by script, scene, quality and project files) are checkpointed so a retry
  resumes instead of starting over. Paragraphs are chunked below Kokoro's 510-phoneme
  limit; never request `continuous` synthesis because the bundled models lack durations.
- Record Python dependencies in pyproject.toml and uv.lock. Rebuild the editor
  with npm run build:editor after changing UI assets.
- Run the development and verification commands below before handing off changes.
- Preserve download pins, native source provenance, original licenses and
  current-release checksums when building standalone releases.
- Do not add secrets, machine-specific absolute paths or obsolete starter files.

## Development and verification

```powershell
uv sync --locked
npm ci
npm run build:editor
uv run python desktop.py
```

Run `uv run ruff check .`, `uv run ruff format --check .`,
`npm run format:check`, and `uv run python -m unittest discover -s tests -v`.
Set MANIM_STUDIO_TEST_KOKORO to a folder holding the Kokoro model files to also run
the real narrated-render test.
Browser checks are `uv run python tests/ui_smoke.py` and
`uv run python tests/react_audit.py`. Install Chromium with
`uv run python -m playwright install chromium`. These checks create isolated
data roots and a real local server with deterministic CLI subprocesses; no paid
agent calls or existing project data are used. Set MANIM_STUDIO_TEST_KOKORO to
also verify generated narration, audio preview and auto-render with real Kokoro.
Screenshots go to a temporary directory, or MANIM_STUDIO_TEST_ARTIFACTS when set.
Set MANIM_STUDIO_UPDATE_SCREENSHOT=1 only when intentionally refreshing the README image.
On an interactive Windows desktop, also run `uv run python tests/native_file_picker.py`
to verify that document/media dialogs appear, select files, and cancel from both
the composer and Assets panel. Ordinary browser checks use the HTML picker fallback.

## Standalone releases

Run the Windows builders in this order: `packaging/build_ffmpeg.py`,
`packaging/prepare_vc.py`, `packaging/build.py`, `packaging/build_online.py`,
and `packaging/collect_sources.py`. Use Python 3.12, uv, Node.js, Inno Setup
and a licensed Visual Studio installation with its release CRT. The VC helper
requires `--source` for the x64 CRT directory and `--visual-studio` for its
licensed installation. `--verify-cache` instead rechecks a previously captured
release CRT and its original licenses without changing provenance.
These tools are for developers; end users run the EXE.

The default build uses pinned Python archives and Microsoft-signed Evergreen
installers in packaging/webview-evergreen.json. Fixed WebView2 remains an explicit
build profile. The minimal TeX build requires the reviewed
`.build-cache/math-runtime` snapshot. Do not bypass native hashes or replace the
snapshot silently. Rebuilding it requires source review and updated inventories.
Run `packaging/verify_native.py`, `verify_math.py`, `verify_typst.py` and `verify_audio.py` with the
bundle's own Python and fresh data. Complete all builders before running native
checks, and run those checks on a separate copy of the bundle: MiKTeX can rewrite
font configuration files at runtime. Verify complete math inventories, ZIP CRCs,
source parity and checksums on the untouched release payload.
Publish matching third-party sources and original license notices with releases.
`espeakng-loader` ships espeak-ng (GPL-3.0) and `phonemizer` is GPL-3.0; keep their
matching sources/recipes pinned in `packaging/source-lock.json` and recheck
`packaging/narration-source-audit.json` whenever narration dependencies change.

Updater checks are included in the Python suite and `tests/react_workspace.py`
(also run by ui_smoke). On Windows, `tests/test_update_handoff.py` compiles the
native helper and launcher into isolated fixtures to verify installation locks,
payload validation, rollback, interrupted recovery and custom data paths.
Run `tests/test_installer.py` with Inno Setup available to exercise silent
replacement of read-only shipped files while preserving user data.
Installer builders inventory payload path lengths; setup rejects unsupported
destinations before deleting or replacing existing files. Online setup also
checks its prepared runtime on retries.
Ship Studio Update.exe with the matching app source; updater downloads must use
the official release URLs and pass SHA256 verification before installation.
Keep the portable recovery journal and installer AppId compatible.

Version 1.0.0 was the first public release; releases ship minimal TeX plus Typst. Publish the intended stable
release as GitHub's latest release. Preserve Git tags and published asset bytes;
retire an older GitHub release only when explicitly requested.
Current source version: 1.1.0. Version metadata and builders read pyproject.toml;
keep runtime.py's fallback version in sync. Project expansion persists in the
manim-project-expanded preference. Deleting the last project creates a fresh blank
project and preserves render history; every UI deletion requires confirmation.

Agent chats can be forked (`/api/projects/<id>/threads/<id>/fork`): the copy replays the
transcript once as context because provider sessions cannot be split. Provider
failures are classified into stable codes (`agents.ERROR_CODES`, e.g. `usage_limit`,
`rate_limited`, `context_too_long`) and shown with a hint in the timeline. Claude models
are listed with explicit versions in `CLAUDE_MODELS`; add new releases there. The sidebar
update notice and onboarding voice step share the updater and narration download APIs.

Keep README.md for users and AGENTS.md for developers. Preserve third-party license files
and notices in dependencies, font assets and release runtimes. The app's code is
MIT licensed, copyright ItsTatsuya. Scripts execute with the user's permissions;
keep the server on loopback and retain its per-session mutation token checks.
