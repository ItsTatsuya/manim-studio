# Smaller runtime builds

The default source build uses shared Evergreen WebView2, a minimal MiKTeX
payload, and Typst 0.15.0. It keeps Python 3.12, Manim Community 0.21.0,
FastAPI/Uvicorn, pywebview, CodeMirror and the Cairo worker.

## Implementation plan

1. Reuse Microsoft's shared Evergreen runtime instead of distributing a private
   browser. Check the official registry locations; install only when missing.
   Pin the Microsoft-signed bootstrapper and offline installer by SHA256.
2. Derive a minimal TeX payload from the existing hash-verified MiKTeX snapshot.
   Retain latex/pdflatex, dvisvgm, Ghostscript, font utilities, package management,
   native import dependencies, dynamically loaded plugins, TeX macros and fonts.
   Remove unused engines, bibliography tools, spelling dictionaries, TeX4ht and
   documentation while retaining original licenses. Record a complete inventory.
3. Include Typst in the locked Python graph. Detect Typst requirements in scripts
   and check them before rendering. Keep Tex/MathTex syntax for existing scripts;
   Typst/MathTypst use Typst syntax and are separate classes.
4. Build isolated candidates, verify native playback, LaTeX, Typst, audio,
   installer upgrades, update transactions and UI checks, then compare file bytes
   with the original release ZIP. Preserve released assets and checksums.

## Build profiles

```powershell
uv sync --locked
npm ci
npm run build:editor
uv run python packaging/build.py --output-dir dist/release-1.1.0
uv run python packaging/build_online.py --output-dir dist/release-1.1.0
uv run python packaging/collect_sources.py --output-dir dist/release-1.1.0

# Smaller edition: Typst included, LaTeX supplied separately if required.
uv run python packaging/build.py --output-dir dist/typst-only --math-profile none
uv run python packaging/build_online.py --output-dir dist/typst-only

# Retain extra TeX engines when custom scripts need them.
uv run python packaging/build.py --output-dir dist/full-tex --math-profile full
```

The original `.build-cache/math-runtime`, `math-snapshot.json`, native source
catalog and native binary pins remain authoritative. `minimal-tex.json` defines
the reviewed selection; the builder verifies every copied file against the
original inventory. Minimal math does not include XeLaTeX, LuaLaTeX, bibliography
processing or general publishing utilities. Use the full profile or external
tools for those workflows. The standard default Manim equation path is retained.

Normal builds only use pinned Evergreen installers. Updating their pins is an
explicit developer action: `uv run python packaging/prepare_webview.py --lock`.
This downloads from Microsoft's links, checks the Microsoft Authenticode signer,
and records the immutable delivery URLs, hashes and certificate details.

## Installation and updates

The portable bundle includes the 1.9 MB Evergreen bootstrapper. If the shared
runtime is absent, the first native launch installs it per-user and needs
internet. The offline installer carries Microsoft's full Evergreen installer as
an installation prerequisite; it is not copied into Studio's app directory.
Neither installer removes an existing private browser until prerequisite
preparation succeeds. The shared runtime remains available for other apps after
Studio is uninstalled and is excluded from Studio's own footprint.

Online setup now embeds the verified selected math snapshot instead of rebuilding
from a mutable MiKTeX mirror. It downloads hash-locked Python packages, including
Typst, and checks the complete math inventory and native versions before replacing
app files. The initial installer download includes the selected TeX payload;
Python packages are downloaded separately during setup.

The portable updater validates explicit browser/math profiles and preserves
UserData during component replacement. Version 1.1.0 is the first public release
and publishes only the minimal TeX plus Typst profile.

## Product screenshot

Start Studio in browser mode with isolated `MANIM_STUDIO_DATA`, then run:

```powershell
$env:MANIM_STUDIO_UPDATE_SCREENSHOT = '1'
uv run python packaging/capture_screenshot.py --url http://127.0.0.1:8765
```

The capture renders a real scene with both equation engines before taking the
image. Keep test screenshots and render data outside the tracked source tree.
