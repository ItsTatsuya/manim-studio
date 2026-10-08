# Manim Studio

Turn Manim Python scripts into animations in one Windows workspace. Edit, render,
and preview videos without installing Python or running commands.

**[Download Manim Studio](https://github.com/ItsTatsuya/manim-studio/releases/latest)**
· [Report a bug](https://github.com/ItsTatsuya/manim-studio/issues)

![Manim Studio showing the Python editor and a rendered LaTeX and Typst animation](docs/images/manim-studio.png)

## Install

Requires **64-bit Windows 10 (2004+) or Windows 11**. Allow 3 GB of free space
during setup. The app is unsigned, so Windows may display a security warning.

| Download                                                                                                                      | Use it when                                                                                 |
| ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [Setup](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.1.0/Manim-Studio-1.1.0-Setup-x64.exe)                 | You have internet. Setup downloads and prepares the rendering engine.                       |
| [Portable ZIP](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.1.0/Manim-Studio-1.1.0-Portable-x64.zip)       | You want a folder you can move. Extract the **entire ZIP**, then open **Manim Studio.exe**. |
| [Offline setup](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.1.0/Manim-Studio-1.1.0-Offline-Setup-x64.exe) | You need to install without downloading rendering tools.                                    |

All downloads include **standard LaTeX and Typst equation support**. Studio uses
shared Microsoft WebView2; portable first launch needs internet only if that
runtime is missing. Standard rendering works offline after setup.

## Make an animation

1. Paste or import a complete **Manim Community** script.
2. Name the animation and choose its scene, resolution, and frame rate.
3. Click **Render animation** or press **Ctrl+Enter**.
4. Preview the result and save the MP4. Find previous work in **My renders**.

The editor includes autocomplete, formatting, and errors linked to script lines.
**Help** replays the guided tour. Scripts run with your Windows account's
permissions; review code before rendering.

## Your files and updates

Installed copies save work in `%LOCALAPPDATA%\ManimStudio\Data`. Portable copies
use `UserData` beside the executable. Upgrades preserve scripts, assets,
preferences, and render history.

Use **Updates** to download and install new releases. For a **v1.0.0 portable
copy**, migrate manually: close Studio, extract the new ZIP into a fresh folder,
and copy your existing `UserData` into it before opening the new app.

## Development

With Python 3.12, [uv](https://docs.astral.sh/uv/), and Node.js installed:

```powershell
uv sync --locked
npm ci
npm run build:editor
uv run python desktop.py
```

See [AGENTS.md](AGENTS.md) for checks and [runtime packaging](docs/runtime-packaging.md)
for build profiles and equation compatibility.

Created by [ItsTatsuya](https://github.com/ItsTatsuya). App code is [MIT licensed](LICENSE).
Built on [Manim Community](https://www.manim.community/) and [FFmpeg](https://ffmpeg.org/).
Third-party licenses remain with their components; matching sources and checksums
are included with each release.
