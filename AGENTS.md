# Manim Studio project contract

This repository contains the standalone Windows GUI application. End users
install it or extract the portable ZIP and open Manim Studio.exe. The original
starter scripts and manual rendering launchers have been retired.

- Keep Manim Community Edition 0.21.0 and Python 3.12; do not use ManimGL or manimlib.
- Preserve the local FastAPI/Uvicorn, pywebview/WebView2 and CodeMirror architecture.
- Use the Cairo renderer unless a requested feature requires a deliberate change.
- Keep rendering in the worker process, with progress, cancellation and diagnostics.
- Keep app code separate from user data. Preserve scripts, preferences, assets and
  render history during updates; never commit them or generated videos.
- Keep named output folders and compatibility with existing GUI render manifests.
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
Browser checks are `tests/ui_smoke.py` and `tests/workspace_audit.py`; start
`desktop.py --browser --no-open --port 8765`, install Chromium with
`uv run python -m playwright install chromium`, and set MANIM_STUDIO_URL to
http://127.0.0.1:8765. Always use an isolated MANIM_STUDIO_DATA for verification.
Screenshots go to a temporary directory, or MANIM_STUDIO_TEST_ARTIFACTS when set.
Set MANIM_STUDIO_UPDATE_SCREENSHOT=1 only when intentionally refreshing the README image.

## Standalone releases

Run the Windows builders in this order: `packaging/build_ffmpeg.py`,
`packaging/prepare_vc.py`, `packaging/build.py`, `packaging/build_online.py`,
and `packaging/collect_sources.py`. Use Python 3.12, uv, Node.js, Inno Setup
and a licensed Visual Studio installation with its release CRT. The VC helper
requires `--source` for the x64 CRT directory and `--visual-studio` for its
licensed installation. These tools are for developers; end users run the EXE.

The offline build requires the pinned Python/WebView2 archives and reviewed
`.build-cache/math-runtime` snapshot. Do not bypass native hashes or replace the
snapshot silently. Rebuilding it requires source review and updated inventories.
Run `packaging/verify_native.py`, `verify_math.py` and `verify_audio.py` with the
bundle's own Python and fresh data. Verify ZIP CRCs, source parity and checksums.
Publish matching third-party sources and original license notices with releases.

Updater checks are included in the Python suite and `tests/workspace_updates.py`
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

Version 1.0.0 is the first public release. Publish the intended stable release
as GitHub's latest release. Preserve published tags and checksums for subsequent
releases.

Keep README.md for users and AGENTS.md for developers. Preserve third-party license files
and notices in dependencies, font assets and release runtimes. The app's code is
MIT licensed, copyright ItsTatsuya. Scripts execute with the user's permissions;
keep the server on loopback and retain its per-session mutation token checks.
