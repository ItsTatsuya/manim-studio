# Manim Studio

Create narrated Manim animations in one Windows workspace. Chat with Codex or
Claude Code to generate the scene, narration and assets, or edit Python directly.
Render and preview videos without installing Python.

**[Download Manim Studio](https://github.com/ItsTatsuya/manim-studio/releases/latest)**
Â· [Report a bug](https://github.com/ItsTatsuya/manim-studio/issues)

![Manim Studio with project chats, Python editor and animation tools](docs/images/manim-studio.png)

**1.0.0** is the first release of Manim Studio, with the agent workspace,
guided setup, document and media uploads, and local narrated rendering.

## Install

Requires **64-bit Windows 10 (2004+) or Windows 11**. Allow 3 GB of free space
during setup. The app is unsigned, so Windows may display a security warning.

| Download                                                                                                                      | Use it when                                                                                 |
| ----------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| [Setup](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.0.0/Manim-Studio-1.0.0-Setup-x64.exe)                 | You have internet. Setup downloads and prepares the rendering engine.                       |
| [Portable ZIP](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.0.0/Manim-Studio-1.0.0-Portable-x64.zip)       | You want a folder you can move. Extract the **entire ZIP**, then open **Manim Studio.exe**. |
| [Offline setup](https://github.com/ItsTatsuya/manim-studio/releases/download/v1.0.0/Manim-Studio-1.0.0-Offline-Setup-x64.exe) | You need to install without downloading rendering tools.                                    |

All downloads include **standard LaTeX and Typst equation support**. Studio uses
shared Microsoft WebView2; portable first launch needs internet only if that
runtime is missing. Standard rendering works offline after setup.

## Make an animation

First launch opens a setup guide that checks for Codex and Claude Code, offers
installation and login, and explains the workspace. You can skip setup to edit
scripts manually, or reopen it from **Settings â†’ General â†’ Open setup**.

1. Pick a project in the sidebar, or select **New project** to start one. Each project
   keeps its own script, narration and assets, and saves automatically.
2. Describe your video in chat, or paste a complete **Manim Community** script in **Script**.
3. In **Preview**, choose a scene and output preset. **Custom** sets exact dimensions and frame rate.
4. Click **Render** or press **Ctrl+Enter**. Agent changes render automatically when enabled in Settings.
5. Preview the result and save the MP4. Find previous work in **My renders**.

The editor includes autocomplete, formatting, and errors linked to script lines.
Scripts and agent commands run with your Windows account's
permissions; review code before rendering.

## Connect Codex or Claude Code

Open **Settings â†’ Providers**. Studio detects locally installed CLIs, shows their
version and login status, and provides the install command and **Log in** action.
The first-run setup can open each CLI's installer in PowerShell. Codex's npm
installation requires Node.js LTS; setup links to its download page.
Complete login in the CLI window, then choose **Check again**. You can override
the binary path, choose a default model and reasoning effort, and add custom model
IDs. Codex's model list comes from the CLI's live catalog.

Each chat belongs to a project and resumes its provider session on later messages.
The timeline streams replies, commands, file changes and errors. **Stop** cancels
the process tree. Start a new chat to change providers after a session is established.
Your existing CLI login stays with the provider; Studio does not store credentials.
Agent generation requires the CLI's account access and internet. The rendering
engine and downloaded voice models work offline.

**Full access** allows file edits and commands without CLI permission prompts.
**Auto edits** uses the Codex workspace sandbox or Claude's acceptEdits permissions;
Claude's mode is not an operating-system sandbox. Agents are instructed to use the
installed libraries, create local assets and run Studio's scene checks.

## Manage projects

Use the arrow beside a project to expand or collapse its chats. Studio remembers
your choices.

Collapse the sidebar into an icon rail with the button beside **Manim Studio**.
Use the paperclip in chat to upload reference documents, images or audio. Uploads
finish before the message can be sent.
The **â‹¯** button on each row opens Rename, Open project folder and Delete, including
for projects you are not currently editing.

Deleting asks for confirmation and removes that project's script, narration and
assets. Finished videos remain in **My renders**. You cannot delete a project while
it is rendering. Deleting the last project opens a new blank project.

Current release: **1.2.1**. See the [release notes](docs/releases/v1.2.1.md).

## Assets

Open the **Assets** tab to add images, SVGs and sounds to the current project.
Refer to them in your script as `assets/<file name>`. Copy the path beside an asset,
or attach reference files to your chat and ask the agent to use them.

## Import an AI project

Select **Import project** in the sidebar, then choose a project folder or ZIP.
Studio creates a separate project, loads the scene and narration, and keeps nested
asset paths intact. Select **Copy project prompt** for instructions you can give
an online AI that creates downloadable files. Review the imported code, download
the voice model if prompted, and select **Render**.

The minimal package is:

```text
My animation/
  script.py
  narration.txt       # optional timed voice-over
  assets/             # only referenced images, fonts or audio
```

Other scene filenames work when the folder contains one scene script. For several
scene files, include `project.json` with `{"name":"My animation","entrypoint":"main.py"}`.
Studio also reads `narration.json` as a list of `{"start":0,"text":"Hello"}`
segments, or an object with `segments`, `voice`, `speed` and `quality` settings.
Existing TSkill chapter lists are supported. Generated previews, installation
helpers and reports are omitted; imported scripts never install dependencies.
Packages can contain up to 2000 files and 512 MB of unpacked content.

## Narration

The **Narration** tab adds a spoken voice-over to the finished video. Speech is
generated on your computer with the [Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M)
voice model, which runs on the CPU without a GPU. Speed depends on your computer.
The first use downloads the model once (about 120 MB for Standard quality,
350 MB for High); after that narration works offline.

Write one paragraph per spoken line. Start a paragraph with a time such as
`[0:05]` to begin speaking at that moment; otherwise it follows the previous
line. Lines starting with `#` are notes. Narration is mixed with any sound your
scene already adds. If the narration is longer than the animation, the last
frame holds; add `self.wait()` at the end of the scene to plan for it.

For a more natural delivery, write short spoken sentences with punctuation and
spell equations out in words. Studio normalizes notation, adds sentence pauses,
softens abrupt audio boundaries and levels loudness. Try different voices and
speeds with **Preview voice**; High quality uses the full-precision model. Naturalness
still depends on the selected voice and wording.

## Your files

Installed copies save work in `%LOCALAPPDATA%\ManimStudio\Data`. Portable copies
use `UserData` beside the executable. Projects (scripts, narration and assets) are
in its `projects` folder; render history, preferences and the voice model stay in
the same data folder. Use **Settings â†’ About & updates** to check for future releases.
Each project stores `script.py`, `narration.txt`, voice settings in `project.json`
and its own `assets/` directory. The project's **â‹¯ â†’ Open folder** action reveals
these files.

Chat sessions and provider preferences stay in `.studio`. Project edits save
automatically; failed saves show a retry action, and recoverable drafts survive a
reload. The desktop window saves pending changes before it closes.

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
