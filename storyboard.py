"""Lesson-plan storyboards: a reviewed teaching script written before any video.

A plan-mode agent turn writes only ``storyboard.md`` in the project or video
workspace. Studio never renders a plan turn. The user reviews and edits the
storyboard, then approves it, which starts an ordinary build turn that turns
the storyboard into ``script.py`` and ``narration.txt``.

Approval is stored in a hidden sidecar as the SHA-256 of the approved text, so
any later edit (by the user or an agent) returns the storyboard to draft.
"""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path

import projects

FILE = projects.STORYBOARD_FILE
STATE = ".storyboard.json"
MAX_TEXT = 200_000
MODES = ("build", "plan")
GUARDED = (projects.SCRIPT_FILE, projects.NARRATION_FILE)

# Sent as the build turn after approval. It points at the file instead of
# pasting it, so the prompt stays small however long the storyboard is.
BUILD_PROMPT = (
    "The storyboard in storyboard.md is approved. Read it in full and build the video "
    "from it: write script.py and narration.txt that follow its scenes in order, with "
    "the stated durations, on-screen directions, diagrams, colours, worked examples, "
    "answers and key spoken points. Turn the spoken points into natural narration "
    "timed to the animation. Do not edit storyboard.md. Check the scene before finishing."
)

# A one-line reminder at the top of every plan turn, for sessions that were
# briefed in build mode earlier.
PLAN_REMINDER = (
    "Plan mode: write or revise only storyboard.md. Do not create or change "
    "script.py, narration.txt or assets, and do not run checks or renders."
)


class StoryboardError(ValueError):
    """A user-facing problem with a storyboard request."""


def instructions(project_dir: Path) -> str:
    """The plan-mode system prompt: the storyboard format, kept compact."""
    return f"""You are the lesson planner inside Manim Studio, a desktop app that turns
lesson plans into narrated Manim explainer videos. PLAN MODE: write the teaching
storyboard for one video into storyboard.md in your working directory:
{project_dir}

Rules
- Create or edit only storyboard.md. Never create or change script.py,
  narration.txt, project.json or assets/. Write no Manim code, run no checks and
  render nothing; the user reviews this plan before any video is made.
- First read the reference files the user attached (assets/, e.g.
  assets/video-script.txt) and keep their content, order and notation.
- When storyboard.md exists, revise it as asked and keep everything else.
- GitHub Markdown only. Write maths inline as code, e.g. `2x + y = 5`.

Format (headings exactly as shown; omit callouts a section does not need)
# <Course> · <Module>
## Video <n.nn> — <Title>
<One-line summary of what the video answers.>
**Builds on:** <prerequisites> · **Length:** ~<m:ss>

### By the end of this video you can
1. <observable objective, starting with a verb>

### <s>. <Section title>
> **Scene <s>** · ~<m:ss> · **On screen:** <objects, layout, colours and how
> they animate in order> · **Say:** <2–4 key spoken points, not the full script>

<Explanation prose in teaching order.>

**Example <n.nn>.<k> — <title>**
*Problem:* <statement>
*Solution:* numbered steps.
**Answer:** `<final result>`

**Diagram:** <axes and ranges, lines/curves with equations, labelled points,
colours, what is highlighted>

> **Key idea:** <explanation> — **`<one-line takeaway>`**

> **Pause and think:** <question> *Answer:* <short answer>

| Condition | Meaning | Picture |   (a ratio/condition table where it fits)

> **Exam tip:** <tip>
> **Common mistakes:** <mistake → correction>

### Practice problems
- **P1.** <problem> (★ marks harder ones, e.g. **P3 ★**)
Answers: P1 <answer>; …

### Scene plan
| Scene | Section | ~Time | On screen | Key spoken points |

Keep each scene short enough to animate (about 20–90 s) and make the times add
up to the length. Finish with a two-line summary and ask the user to review the
Storyboard tab, then approve it to generate the video."""


def _hash(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _state(folder: Path) -> dict:
    try:
        value = json.loads((folder / STATE).read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return {}
    return value if isinstance(value, dict) else {}


def read(folder: Path) -> dict:
    """The storyboard and whether the current text is the approved one."""
    path = folder / FILE
    try:
        text = path.read_text(encoding="utf-8")[:MAX_TEXT]
        updated = datetime.fromtimestamp(path.stat().st_mtime, timezone.utc).isoformat()
    except (OSError, UnicodeError):
        return {"text": "", "status": "none", "approved": None, "updated": None}
    state = _state(folder)
    approved = bool(text.strip()) and state.get("hash") == _hash(text)
    return {
        "text": text,
        "status": "approved" if approved else "draft",
        "approved": state.get("approved") if approved else None,
        "updated": updated,
    }


def save(folder: Path, text: str) -> dict:
    if not isinstance(text, str) or len(text) > MAX_TEXT:
        raise StoryboardError("Keep the storyboard under 200,000 characters.")
    with projects.LOCK:
        projects._write_atomic(folder / FILE, text)
    return read(folder)


def approve(folder: Path) -> dict:
    current = read(folder)
    if not current["text"].strip():
        raise StoryboardError("Write or generate a storyboard before approving it.")
    if current["status"] != "approved":
        with projects.LOCK:
            projects._write_atomic(
                folder / STATE,
                json.dumps(
                    {
                        "hash": _hash(current["text"]),
                        "approved": datetime.now(timezone.utc).isoformat(),
                    }
                ),
            )
    return read(folder)


def stamp(folder: Path):
    """A cheap change marker for storyboard.md, or None when it is missing."""
    try:
        stat = (folder / FILE).stat()
    except OSError:
        return None
    return stat.st_mtime_ns, stat.st_size


def guard(folder: Path) -> dict:
    """Remember the video files a plan turn must not change."""
    saved = {}
    for name in GUARDED:
        path = folder / name
        try:
            saved[name] = (path.read_bytes(), path.stat().st_mtime_ns)
        except FileNotFoundError:
            saved[name] = None
        except OSError:
            continue
    return saved


def restore(folder: Path, saved: dict) -> list[str]:
    """Undo plan-turn edits to the video files; returns what was restored.

    The original modification time comes back too, so the turn does not
    report the restored files as changed.
    """
    restored = []
    for name, original in saved.items():
        path = folder / name
        try:
            current = path.read_bytes()
        except FileNotFoundError:
            current = None
        except OSError:
            continue
        if current == (original and original[0]):
            continue
        try:
            if original is None:
                path.unlink(missing_ok=True)
            else:
                data, modified = original
                temporary = path.with_name(path.name + ".tmp")
                temporary.write_bytes(data)
                temporary.replace(path)
                os.utime(path, ns=(modified, modified))
            restored.append(name)
        except OSError:
            continue
    return restored
