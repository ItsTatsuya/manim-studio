"""House styles: named looks shared by every video in a project.

Styles are user data in ``DATA_ROOT/.studio/styles.json``. Built-in presets are
read-only (they can be duplicated). A project stores only a style id in its
``project.json``; a PDF-batch video without one follows its parent project and
anything unknown falls back to the default style.

Studio applies a style twice: a short guide is added to the agent instructions,
and a generated ``studio_style.py`` beside ``script.py`` gives scripts the same
colours, fonts, sizes and helpers. The module is only rewritten when its text
changes, so the render cache keeps hitting while the style is unchanged.
"""

from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import re
import struct
import threading
import time
import uuid

STYLE_FILE = "styles.json"
MODULE_FILE = "studio_style.py"
MAX_STYLES = 100
MAX_RULES = 2000
FONT_SUFFIXES = {".ttf", ".otf", ".ttc"}
MAX_FONT_FILES = 64
LOCK = threading.RLock()
ID = re.compile(r"[a-z0-9][a-z0-9-]{0,63}")
HEX = re.compile(r"#[0-9a-fA-F]{6}")
FONT = re.compile(r"[^\x00-\x1f\"'\\{}<>]{0,80}")
COLORS = (
    "background",
    "text",
    "muted",
    "primary",
    "secondary",
    "accent",
    "positive",
    "negative",
    "key_fill",
    "example_fill",
)
SIZES = {"title": (20, 120), "heading": (16, 100), "body": (12, 80), "small": (8, 60)}
TITLE_PLACEMENTS = ("top-left", "top-center", "center")
MATH = ("auto", "typst", "tex")


class StyleError(Exception):
    """A user-facing problem with a style request."""


BUILTIN = [
    {
        "id": "clean",
        "name": "Clean",
        "description": "Crisp white slides set in Inter with an indigo accent.",
        "fonts": {
            "heading": "Inter",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#FFFFFF",
            "text": "#111827",
            "muted": "#6B7280",
            "primary": "#4F46E5",
            "secondary": "#0284C7",
            "accent": "#E11D48",
            "positive": "#059669",
            "negative": "#DC2626",
            "key_fill": "#EEF2FF",
            "example_fill": "#F0F9FF",
        },
        "sizes": {"title": 50, "heading": 38, "body": 30, "small": 22},
        "layout": {"margin": 0.6, "title": "top-left"},
        "pacing": {"run_time": 1.0, "wait": 1.0, "words_per_second": 2.4},
        "voice": "Warm, clear and confident, like a favourite teacher.",
        "rules": (
            "One idea per slide. Introduce a term before using it and highlight it "
            "in PRIMARY. Finish with a one-line takeaway."
        ),
    },
    {
        "id": "editorial",
        "name": "Editorial",
        "description": "Magazine feel: Lora headlines on cream with terracotta and forest.",
        "fonts": {
            "heading": "Lora",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#F7F3EA",
            "text": "#27221C",
            "muted": "#7A7064",
            "primary": "#C2410C",
            "secondary": "#166534",
            "accent": "#9F1239",
            "positive": "#166534",
            "negative": "#B91C1C",
            "key_fill": "#F4E2D2",
            "example_fill": "#E2EDDB",
        },
        "sizes": {"title": 56, "heading": 40, "body": 30, "small": 22},
        "layout": {"margin": 0.7, "title": "top-left"},
        "pacing": {"run_time": 1.2, "wait": 1.2, "words_per_second": 2.3},
        "voice": "Thoughtful storyteller who ties ideas to the real world.",
        "rules": (
            "Open with a question or a real-world hook. One striking visual per slide "
            "and few words."
        ),
    },
    {
        "id": "textbook",
        "name": "Textbook",
        "description": "Warm paper, Source Serif headings and teal and amber highlights.",
        "fonts": {
            "heading": "Source Serif 4",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#FBFAF7",
            "text": "#1C1917",
            "muted": "#78716C",
            "primary": "#0F766E",
            "secondary": "#B45309",
            "accent": "#BE123C",
            "positive": "#15803D",
            "negative": "#B91C1C",
            "key_fill": "#E3F2EF",
            "example_fill": "#FBF0DC",
        },
        "sizes": {"title": 54, "heading": 40, "body": 30, "small": 22},
        "layout": {"margin": 0.6, "title": "top-left"},
        "pacing": {"run_time": 1.0, "wait": 1.2, "words_per_second": 2.3},
        "voice": "Calm, clear teacher: precise, friendly and unhurried.",
        "rules": (
            "Open with the learning objectives. Put definitions and key results in "
            "key boxes and worked examples in example boxes. End with a short recap."
        ),
    },
    {
        "id": "dark",
        "name": "Dark explainer",
        "description": "Deep navy with sky blue and gold, built for visual intuition.",
        "fonts": {
            "heading": "Inter",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#0F172A",
            "text": "#E2E8F0",
            "muted": "#94A3B8",
            "primary": "#38BDF8",
            "secondary": "#FACC15",
            "accent": "#FB7185",
            "positive": "#4ADE80",
            "negative": "#F87171",
            "key_fill": "#132A45",
            "example_fill": "#2B2614",
        },
        "sizes": {"title": 54, "heading": 40, "body": 32, "small": 24},
        "layout": {"margin": 0.6, "title": "top-center"},
        "pacing": {"run_time": 1.5, "wait": 1.2, "words_per_second": 2.3},
        "voice": "Curious, conversational storyteller who builds intuition.",
        "rules": (
            "Build intuition visually before showing formulas. Prefer animated "
            "transformations to hard cuts."
        ),
    },
    {
        "id": "midnight",
        "name": "Midnight",
        "description": "Near-black and quiet, with one violet accent and generous space.",
        "fonts": {
            "heading": "Inter",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#0A0A0B",
            "text": "#F4F4F5",
            "muted": "#A1A1AA",
            "primary": "#A78BFA",
            "secondary": "#5EEAD4",
            "accent": "#F472B6",
            "positive": "#86EFAC",
            "negative": "#FCA5A5",
            "key_fill": "#1C1830",
            "example_fill": "#10251F",
        },
        "sizes": {"title": 52, "heading": 38, "body": 30, "small": 22},
        "layout": {"margin": 0.7, "title": "top-left"},
        "pacing": {"run_time": 1.2, "wait": 1.2, "words_per_second": 2.3},
        "voice": "Relaxed and thoughtful, with room to breathe.",
        "rules": (
            "Show very little at once: one shape or formula and a few words. Use "
            "PRIMARY only for the thing the viewer should look at."
        ),
    },
    {
        "id": "blueprint",
        "name": "Blueprint",
        "description": "Engineering look: Space Grotesk and cyan lines on deep blue.",
        "fonts": {
            "heading": "Space Grotesk",
            "body": "Inter",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#0B1D33",
            "text": "#E6F1FF",
            "muted": "#8BA3C0",
            "primary": "#5EEAD4",
            "secondary": "#93C5FD",
            "accent": "#FDE047",
            "positive": "#86EFAC",
            "negative": "#FCA5A5",
            "key_fill": "#11304F",
            "example_fill": "#16324A",
        },
        "sizes": {"title": 52, "heading": 38, "body": 30, "small": 22},
        "layout": {"margin": 0.6, "title": "top-left"},
        "pacing": {"run_time": 1.2, "wait": 1.0, "words_per_second": 2.4},
        "voice": "Precise engineer walking through a design, step by step.",
        "rules": (
            "Prefer diagrams, arrows and measurements to prose. Tag parts with short "
            "code_text labels."
        ),
    },
    {
        "id": "high-contrast",
        "name": "High contrast",
        "description": "Atkinson Hyperlegible, large type and strong colours for accessibility.",
        "fonts": {
            "heading": "Atkinson Hyperlegible Next",
            "body": "Atkinson Hyperlegible Next",
            "code": "JetBrains Mono",
            "math": "auto",
        },
        "colors": {
            "background": "#000000",
            "text": "#FFFFFF",
            "muted": "#D0D0D0",
            "primary": "#FFD400",
            "secondary": "#00E5FF",
            "accent": "#FF4FD8",
            "positive": "#4CFF4C",
            "negative": "#FF5A5A",
            "key_fill": "#1A1A1A",
            "example_fill": "#0D1B2A",
        },
        "sizes": {"title": 60, "heading": 46, "body": 36, "small": 28},
        "layout": {"margin": 0.6, "title": "top-left"},
        "pacing": {"run_time": 1.2, "wait": 2.0, "words_per_second": 2.0},
        "voice": "Slow, clear and steady.",
        "rules": (
            "Never rely on colour alone: label everything. Keep at most three "
            "items on screen at once."
        ),
    },
]
BUILTIN_IDS = {style["id"] for style in BUILTIN}
DEFAULT_ID = "clean"
# Open-licence fonts shipped with Studio (fonts/SOURCES.json lists their origin),
# so every built-in style renders the same on any computer.
FONT_DIR = Path(__file__).resolve().parent / "fonts"


def bundled_fonts() -> list[dict]:
    """The font files Studio ships, with the family name scripts use."""
    try:
        listed = json.loads((FONT_DIR / "SOURCES.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return []
    return [
        {"family": item["family"], "file": item["file"]}
        for item in listed.get("fonts", [])
        if isinstance(item, dict)
        and isinstance(item.get("family"), str)
        and isinstance(item.get("file"), str)
        and (FONT_DIR / item["file"]).is_file()
    ]


def configure_fonts(data_root: Path) -> None:
    """Have Pango find fonts through fontconfig, Studio's own fonts included.

    On Windows ManimPango re-adds every font registered with the native API
    to Pango for each Text, about half a second per font file. Fontconfig
    reads Studio's fonts, the system's and the user's once from a cached scan.
    Call before Pango loads; child processes inherit the environment.
    """
    if os.name != "nt" or os.environ.get("PANGOCAIRO_BACKEND"):
        return
    from xml.sax.saxutils import escape

    folder = data_root / ".studio" / "fontconfig"
    windows = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Fonts"
    user = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "Microsoft"
    dirs = [windows, user / "Windows" / "Fonts", FONT_DIR]
    text = (
        '<?xml version="1.0"?>\n'
        '<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">\n<fontconfig>\n'
        + "".join(f"  <dir>{escape(str(path))}</dir>\n" for path in dirs)
        + f"  <cachedir>{escape(str(folder / 'cache'))}</cachedir>\n</fontconfig>\n"
    )
    config = folder / "fonts.conf"
    try:
        if not config.is_file() or config.read_text(encoding="utf-8") != text:
            _write_atomic(config, text)
    except OSError:
        return
    os.environ["FONTCONFIG_FILE"] = str(config)
    os.environ["PANGOCAIRO_BACKEND"] = "fc"


def _write_atomic(path: Path, text: str):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"{path.name}.{uuid.uuid4().hex[:8]}.tmp")
    temporary.write_text(text, encoding="utf-8", newline="\n")
    temporary.replace(path)


def _text(value, limit: int, fallback: str = "") -> str:
    if not isinstance(value, str):
        return fallback
    value = re.sub(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]", "", value).strip()
    return value[:limit]


def _number(value, low: float, high: float, fallback: float) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return fallback
    if value != value or not low <= value <= high:
        raise StyleError(f"Use a value between {low:g} and {high:g}.")
    return round(float(value), 3)


def clean_style(value, base: dict | None = None) -> dict:
    """Validate a style, filling anything missing from ``base`` (or the default)."""
    if not isinstance(value, dict):
        raise StyleError("A style must be an object.")
    base = copy.deepcopy(base or BUILTIN[0])
    style = {
        "name": " ".join(_text(value.get("name"), 60).split()) or base["name"],
        "description": _text(
            value.get("description"), 200, base.get("description", "")
        ),
    }
    fonts = value.get("fonts") if isinstance(value.get("fonts"), dict) else {}
    style["fonts"] = {}
    for key in ("heading", "body", "code"):
        font = fonts.get(key, base["fonts"][key])
        if not isinstance(font, str) or not FONT.fullmatch(font.strip()):
            raise StyleError("Font names can't contain quotes or control characters.")
        style["fonts"][key] = font.strip()
    math = fonts.get("math", base["fonts"].get("math", "auto"))
    style["fonts"]["math"] = math if math in MATH else "auto"
    colors = value.get("colors") if isinstance(value.get("colors"), dict) else {}
    style["colors"] = {}
    for key in COLORS:
        color = colors.get(key, base["colors"][key])
        if not isinstance(color, str) or not HEX.fullmatch(color):
            raise StyleError("Colours must be hex values such as #1F2933.")
        style["colors"][key] = color.upper()
    sizes = value.get("sizes") if isinstance(value.get("sizes"), dict) else {}
    style["sizes"] = {
        key: int(_number(sizes.get(key), low, high, base["sizes"][key]))
        for key, (low, high) in SIZES.items()
    }
    layout = value.get("layout") if isinstance(value.get("layout"), dict) else {}
    style["layout"] = {
        "margin": _number(layout.get("margin"), 0.1, 2.0, base["layout"]["margin"]),
        "title": layout.get("title")
        if layout.get("title") in TITLE_PLACEMENTS
        else base["layout"]["title"],
    }
    pacing = value.get("pacing") if isinstance(value.get("pacing"), dict) else {}
    style["pacing"] = {
        "run_time": _number(
            pacing.get("run_time"), 0.2, 6.0, base["pacing"]["run_time"]
        ),
        "wait": _number(pacing.get("wait"), 0.0, 6.0, base["pacing"]["wait"]),
        "words_per_second": _number(
            pacing.get("words_per_second"),
            1.0,
            4.0,
            base["pacing"]["words_per_second"],
        ),
    }
    style["voice"] = _text(value.get("voice"), 200, base["voice"])
    style["rules"] = _text(value.get("rules"), MAX_RULES, base["rules"])
    return style


# ---------------------------------------------------------------- library


def _path(data_root: Path) -> Path:
    return data_root / ".studio" / STYLE_FILE


def _load(data_root: Path) -> dict:
    try:
        saved = json.loads(_path(data_root).read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        saved = {}
    if not isinstance(saved, dict):
        saved = {}
    styles = []
    for item in (
        saved.get("styles", [])[:MAX_STYLES]
        if isinstance(saved.get("styles"), list)
        else []
    ):
        if (
            not isinstance(item, dict)
            or not isinstance(item.get("id"), str)
            or not ID.fullmatch(item["id"])
            or item["id"] in BUILTIN_IDS
            or any(style["id"] == item["id"] for style in styles)
        ):
            continue
        try:
            styles.append({"id": item["id"], **clean_style(item)})
        except StyleError:
            continue
    default = saved.get("default")
    ids = BUILTIN_IDS | {style["id"] for style in styles}
    return {
        "default": default if default in ids else DEFAULT_ID,
        "styles": styles,
    }


def _store(data_root: Path, library: dict):
    _write_atomic(_path(data_root), json.dumps(library, indent=2))


def library(data_root: Path) -> dict:
    """Every style, built-ins first, plus the default id."""
    with LOCK:
        saved = _load(data_root)
    return {
        "default": saved["default"],
        "styles": [{**copy.deepcopy(style), "builtin": True} for style in BUILTIN]
        + [{**style, "builtin": False} for style in saved["styles"]],
    }


def get(data_root: Path, style_id: str | None) -> dict:
    """A style by id; unknown or missing ids give the default style."""
    styles = library(data_root)
    by_id = {style["id"]: style for style in styles["styles"]}
    return by_id.get(style_id or "") or by_id[styles["default"]]


def create(data_root: Path, value, source: str | None = None) -> dict:
    with LOCK:
        saved = _load(data_root)
        if len(saved["styles"]) >= MAX_STYLES:
            raise StyleError(f"Keep up to {MAX_STYLES} styles. Delete one first.")
        base = None
        if source is not None:
            base = {style["id"]: style for style in BUILTIN + saved["styles"]}.get(
                source
            )
            if base is None:
                raise KeyError(source)
        style = clean_style(value if isinstance(value, dict) else {}, base)
        if source is not None and not (
            isinstance(value, dict) and _text(value.get("name"), 60)
        ):
            style["name"] = f"{base['name'][:52]} (copy)"
        style = {"id": f"custom-{uuid.uuid4().hex[:12]}", **style}
        saved["styles"].append(style)
        _store(data_root, saved)
    return {**style, "builtin": False}


def update(data_root: Path, style_id: str, value) -> dict:
    if style_id in BUILTIN_IDS:
        raise StyleError("Built-in styles can't be changed. Duplicate it instead.")
    with LOCK:
        saved = _load(data_root)
        for index, style in enumerate(saved["styles"]):
            if style["id"] == style_id:
                merged = {**style, **(value if isinstance(value, dict) else {})}
                saved["styles"][index] = {"id": style_id, **clean_style(merged, style)}
                _store(data_root, saved)
                return {**saved["styles"][index], "builtin": False}
    raise KeyError(style_id)


def delete(data_root: Path, style_id: str) -> dict:
    if style_id in BUILTIN_IDS:
        raise StyleError("Built-in styles can't be deleted.")
    with LOCK:
        saved = _load(data_root)
        kept = [style for style in saved["styles"] if style["id"] != style_id]
        if len(kept) == len(saved["styles"]):
            raise KeyError(style_id)
        saved["styles"] = kept
        if saved["default"] == style_id:
            saved["default"] = DEFAULT_ID
        _store(data_root, saved)
    return library(data_root)


def set_default(data_root: Path, style_id: str) -> dict:
    with LOCK:
        saved = _load(data_root)
        if style_id not in BUILTIN_IDS | {style["id"] for style in saved["styles"]}:
            raise KeyError(style_id)
        saved["default"] = style_id
        _store(data_root, saved)
    return library(data_root)


def valid_id(value) -> bool:
    return isinstance(value, str) and bool(ID.fullmatch(value))


# ---------------------------------------------------------------- projects


def _metadata(folder: Path) -> dict:
    try:
        data = json.loads((folder / "project.json").read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return {}
    return data if isinstance(data, dict) else {}


def _project_style(folder: Path) -> str | None:
    value = _metadata(folder).get("style")
    return value if valid_id(value) else None


def _project_fonts(folder: Path) -> dict:
    value = _metadata(folder).get("fonts")
    if not isinstance(value, dict):
        return {}
    return {
        role: value[role].strip()
        for role in ("heading", "body", "code")
        if isinstance(value.get(role), str)
        and value[role].strip()
        and FONT.fullmatch(value[role].strip())
    }


def for_folder(data_root: Path, folder: Path) -> dict:
    """The style a project or video workspace folder uses, with its own fonts."""
    style_id = _project_style(folder)
    fonts = _project_fonts(folder)
    if folder.parent.name == "videos":
        # PDF-batch videos follow their parent project unless they pick one.
        parent = folder.parent.parent
        if style_id is None:
            style_id = _project_style(parent)
        fonts = {**_project_fonts(parent), **fonts}
    style = get(data_root, style_id)
    if fonts:
        style = {**style, "fonts": {**style["fonts"], **fonts}}
    return style


def _python(value) -> str:
    return json.dumps(value, ensure_ascii=False)


def module_source(style: dict, families: list[str] | None = None) -> str:
    """The generated helper that scripts import as ``from studio_style import *``."""
    shipped = bundled_fonts()
    project_families = _python(
        sorted(set(families or []) | {font["family"] for font in shipped})
    )
    shipped_files = _python([str(FONT_DIR / font["file"]) for font in shipped])
    colors, fonts, sizes = style["colors"], style["fonts"], style["sizes"]
    constants = "\n".join(
        [
            f"STYLE_NAME = {_python(style['name'])}",
            *(f"{key.upper()} = {_python(colors[key])}" for key in COLORS[1:]),
            f"BG = {_python(colors['background'])}",
            f"FONT_HEADING = _family({_python(fonts['heading'])}, 'Sans')",
            f"FONT_BODY = _family({_python(fonts['body'])}, 'Sans')",
            f"FONT_CODE = _family({_python(fonts['code'])}, 'Monospace')",
            f"MATH = {_python(fonts['math'])}",
            *(f"{key.upper()}_SIZE = {sizes[key]}" for key in SIZES),
            f"MATH_SIZE = {round(sizes['body'] * 1.3)}",
            f"MARGIN = {style['layout']['margin']!r}",
            f"TITLE_POSITION = {_python(style['layout']['title'])}",
            f"RUN_TIME = {style['pacing']['run_time']!r}",
            f"WAIT = {style['pacing']['wait']!r}",
            f"WORDS_PER_SECOND = {style['pacing']['words_per_second']!r}",
        ]
    )
    return f'''"""House style "{re.sub(r"[\\\"]", "'", style["name"])}" for this project.

Generated by Manim Studio; edits are overwritten when the style changes. Use:

    from manim import *
    from studio_style import *

    class Lesson(Scene):
        def construct(self):
            apply(self)
            self.play(Write(title("Learning objectives")), run_time=RUN_TIME)
"""

from contextlib import ExitStack
from pathlib import Path

from manim import (
    DOWN,
    LEFT,
    ORIGIN,
    FadeIn,
    FadeOut,
    MEDIUM,
    SEMIBOLD,
    ManimColor,
    UL,
    UP,
    SurroundingRectangle,
    Text,
    VGroup,
    Write,
    config,
)
from manim.mobject.text.text_mobject import register_font

_HERE = Path(__file__).resolve().parent
_FONTS = ExitStack()


# Fonts Studio ships with every style, then any uploaded to this project.
_STUDIO_FONTS = {shipped_files}


def _register_fonts():
    """Make Studio's fonts and those uploaded to this project's assets usable.

    Through fontconfig when Studio set it up (or on Linux): fonts are added
    once. The native Windows route costs about half a second per font for
    every Text, so it is only the fallback.
    """
    import os
    import sys

    import manimpango

    found = [Path(path) for path in _STUDIO_FONTS if Path(path).is_file()]
    assets = _HERE / "assets"
    if assets.is_dir():
        for path in sorted(assets.rglob("*")):
            if path.suffix.lower() in {sorted(FONT_SUFFIXES)!r} and path.is_file():
                found.append(path)
                if len(found) >= {MAX_FONT_FILES + 16}:
                    break
    fontconfig = os.environ.get("PANGOCAIRO_BACKEND") == "fc" or sys.platform.startswith(
        "linux"
    )
    for path in found:
        try:
            if fontconfig:
                manimpango.fc_register_font(str(path))
            else:
                _FONTS.enter_context(register_font(str(path)))
        except Exception:
            pass


_PROJECT_FAMILIES = {project_families}
_INSTALLED = []


def _family(name, fallback):
    """Use the font if it is installed or uploaded, otherwise a safe default."""
    if not name or name in _PROJECT_FAMILIES:
        return name
    try:
        if not _INSTALLED:
            import manimpango

            _INSTALLED.extend(manimpango.list_fonts())
        return name if name in _INSTALLED else fallback
    except Exception:
        return name


_register_fonts()

{constants}

_applied = False


def apply(scene=None):
    """Use this style's background and default colours and fonts."""
    global _applied
    config.background_color = BG
    if scene is not None:
        scene.camera.background_color = ManimColor(BG)
    if _applied:
        return
    _applied = True
    import manim

    for name in ("Text", "MarkupText"):
        try:
            getattr(manim, name).set_default(color=TEXT, font=FONT_BODY)
        except Exception:
            pass
    for name in (
        "VMobject",
        "Rectangle",
        "Dot",
        "MathTex",
        "Tex",
        "MathTypst",
        "Typst",
    ):
        try:
            getattr(manim, name).set_default(color=TEXT)
        except Exception:
            pass


def place_title(mobject):
    if TITLE_POSITION == "top-left":
        return mobject.to_corner(UL, buff=MARGIN)
    if TITLE_POSITION == "center":
        return mobject.move_to(ORIGIN)
    return mobject.to_edge(UP, buff=MARGIN)


def title(text, **options):
    """A slide title in the heading font, placed where the style puts titles."""
    options = {{"font": FONT_HEADING, "font_size": TITLE_SIZE, "color": TEXT, "weight": SEMIBOLD, **options}}
    return place_title(Text(text, **options))


def heading(text, **options):
    options = {{"font": FONT_HEADING, "font_size": HEADING_SIZE, "color": PRIMARY, "weight": MEDIUM, **options}}
    return Text(text, **options)


def body(text, **options):
    options = {{"font": FONT_BODY, "font_size": BODY_SIZE, "color": TEXT, **options}}
    return Text(text, **options)


def caption(text, **options):
    options = {{"font": FONT_BODY, "font_size": SMALL_SIZE, "color": MUTED, **options}}
    return Text(text, **options)


def code_text(text, **options):
    options = {{"font": FONT_CODE, "font_size": SMALL_SIZE, "color": TEXT, **options}}
    return Text(text, **options)


def _box(mobject, fill, stroke, label):
    frame = SurroundingRectangle(
        mobject,
        color=stroke,
        buff=0.3,
        corner_radius=0.16,
        stroke_width=2.5,
        fill_color=fill,
        fill_opacity=1,
    )
    group = VGroup(frame, mobject)
    if label:
        tag = Text(label, font=FONT_HEADING, font_size=SMALL_SIZE, color=stroke)
        tag.next_to(frame, UP, buff=0.1, aligned_edge=LEFT)
        group.add(tag)
    return group


def key_box(mobject, label=None):
    """Frame a definition or key result."""
    return _box(mobject, KEY_FILL, PRIMARY, label)


def example_box(mobject, label=None):
    """Frame a worked example."""
    return _box(mobject, EXAMPLE_FILL, SECONDARY, label)


def bullets(*lines, **options):
    """Left-aligned bullet points in the body style."""
    items = VGroup(*(body("\\u2022 " + line, **options) for line in lines))
    return items.arrange(DOWN, aligned_edge=LEFT, buff=0.35)


_TITLE_HEIGHT = []


def content_box():
    """(left, right, bottom, top) of the area below the title, inside MARGIN."""
    width, height = config.frame_width, config.frame_height
    top = height / 2 - MARGIN
    if TITLE_POSITION != "center":
        if not _TITLE_HEIGHT:
            _TITLE_HEIGHT.append(title("Ag").height)
        top -= _TITLE_HEIGHT[0] + 0.45
    return -width / 2 + MARGIN, width / 2 - MARGIN, -height / 2 + MARGIN, top


def fit(mobject, grow=False):
    """Centre a slide's content in the space below the title, shrunk to fit.

    With grow=True small content is also enlarged (up to 1.5x) to use the space.
    """
    left, right, bottom, top = content_box()
    if mobject.width and mobject.height:
        factor = min((right - left) / mobject.width, (top - bottom) / mobject.height)
        if factor < 1 or grow:
            mobject.scale(min(factor, 1.5 if grow else 1))
    return mobject.move_to([(left + right) / 2, (bottom + top) / 2, 0])


def _items(value):
    if value is None:
        return []
    return [item for item in value if item is not None] if isinstance(value, (list, tuple)) else [value]


def swap(scene, old, new, run_time=None):
    """Fade `old` out, then `new` in, so the two never overlap on screen."""
    run_time = RUN_TIME if run_time is None else run_time
    olds = _items(old)
    if olds:
        scene.play(*(FadeOut(item) for item in olds), run_time=run_time * 0.4)
    scene.play(*(FadeIn(item, shift=UP * 0.15) for item in _items(new)), run_time=run_time * 0.6)
    return new


def new_slide(scene, text=None, run_time=None, keep=()):
    """Clear the slide (all but `keep`) and show a new title; returns the title."""
    run_time = RUN_TIME if run_time is None else run_time
    kept = set(map(id, _items(list(keep))))
    leaving = [item for item in scene.mobjects if id(item) not in kept]
    if leaving:
        scene.play(*(FadeOut(item) for item in leaving), run_time=run_time * 0.4)
    if not text:
        return None
    heading_text = title(text)
    scene.play(Write(heading_text), run_time=run_time * 0.6)
    return heading_text


__all__ = [
    "STYLE_NAME", "BG", {", ".join(_python(key.upper()) for key in COLORS[1:])},
    "FONT_HEADING", "FONT_BODY", "FONT_CODE", "MATH",
    {", ".join(_python(key.upper() + "_SIZE") for key in SIZES)},
    "MATH_SIZE", "MARGIN", "TITLE_POSITION", "RUN_TIME", "WAIT", "WORDS_PER_SECOND",
    "apply", "place_title", "title", "heading", "body", "caption", "code_text",
    "key_box", "example_box", "bullets", "content_box", "fit", "swap", "new_slide",
]
'''


def write_module(data_root: Path, folder: Path) -> dict:
    """Write ``studio_style.py`` into a project folder when its text changed."""
    style = for_folder(data_root, folder)
    text = module_source(style, [font["family"] for font in project_fonts(folder)])
    path = folder / MODULE_FILE
    try:
        current = path.read_text(encoding="utf-8")
    except (OSError, UnicodeError):
        current = None
    if current != text and not path.is_symlink():
        _write_atomic(path, text)
    return style


def guide(style: dict) -> str:
    """A compact style brief for the agent's system instructions."""
    colors, fonts, sizes = style["colors"], style["fonts"], style["sizes"]
    pacing = style["pacing"]
    math = {
        "typst": "\n- Write equations with MathTypst.",
        "tex": "\n- Write equations with MathTex when LaTeX is available.",
    }.get(fonts["math"], "")
    rules = f"\n- House rules: {style['rules']}" if style["rules"] else ""
    voice = f"\n- Narration voice: {style['voice']}" if style["voice"] else ""
    palette = ", ".join(
        f"{key.upper() if key != 'background' else 'BG'} {colors[key]}"
        for key in COLORS
    )
    return f"""House style "{style["name"]}" (keep every video consistent)
- Studio generates studio_style.py beside script.py; never edit it. Add
  `from studio_style import *` after `from manim import *` and call apply(self)
  first in construct() (background, default colours and fonts).
- Use its constants, not literal colours, fonts or sizes: {palette};
  FONT_HEADING {fonts["heading"]}, FONT_BODY {fonts["body"]}, FONT_CODE {fonts["code"]};
  TITLE_SIZE {sizes["title"]}, HEADING_SIZE {sizes["heading"]}, BODY_SIZE {sizes["body"]}, SMALL_SIZE {sizes["small"]}.
- Helpers: title(text) ({style["layout"]["title"]}), heading, body, caption, code_text,
  bullets(*lines), key_box(mob, label), example_box(mob, label). Keep MARGIN
  ({style["layout"]["margin"]}) clear at frame edges. Equations use font_size=MATH_SIZE.
- Layout: place each slide's content as one VGroup with fit(group) (centred
  below the title; grow=True fills sparse slides). Labels sit outside shapes.
- Transitions: new_slide(self, "Title") clears the slide for the next title;
  swap(self, old, new) replaces content without overlap. Both take run_time.
- Pacing: run_time=RUN_TIME ({pacing["run_time"]}s), self.wait(WAIT) ({pacing["wait"]}s);
  narration about {pacing["words_per_second"]} words per second.{math}{voice}{rules}"""


def prepare(data_root: Path, folder: Path) -> str:
    """Refresh the helper module and return the agent's style guide."""
    return guide(write_module(data_root, folder))


# ---------------------------------------------------------------- fonts

_SYSTEM_FONTS: dict = {"checked": 0.0, "fonts": []}


def system_fonts() -> list[str]:
    """Font families Pango can use on this computer (cached for a minute)."""
    with LOCK:
        if time.monotonic() - _SYSTEM_FONTS["checked"] < 60 and _SYSTEM_FONTS["fonts"]:
            return list(_SYSTEM_FONTS["fonts"])
    try:
        import manimpango

        fonts = sorted(
            {name for name in manimpango.list_fonts() if isinstance(name, str)},
            key=str.casefold,
        )
    except Exception:
        fonts = []
    with LOCK:
        _SYSTEM_FONTS.update(checked=time.monotonic(), fonts=fonts)
    return list(fonts)


def font_family(path: Path) -> str | None:
    """Read the family name from a TrueType/OpenType (or first TTC) font."""
    try:
        with path.open("rb") as stream:
            data = stream.read(4 * 1024**2)
    except OSError:
        return None
    try:
        offset = 0
        if data[:4] == b"ttcf":
            offset = struct.unpack(">I", data[12:16])[0]
        tables = struct.unpack(">H", data[offset + 4 : offset + 6])[0]
        for index in range(min(tables, 64)):
            entry = offset + 12 + index * 16
            tag, _, start, _ = struct.unpack(">4sIII", data[entry : entry + 16])
            if tag != b"name":
                continue
            _, count, strings = struct.unpack(">HHH", data[start : start + 6])
            names = {}
            for record in range(min(count, 512)):
                at = start + 6 + record * 12
                platform, _, language, name_id, length, where = struct.unpack(
                    ">HHHHHH", data[at : at + 12]
                )
                if name_id not in (1, 16):
                    continue
                raw = data[start + strings + where : start + strings + where + length]
                if platform in (0, 3):
                    text = raw.decode("utf-16-be", errors="ignore")
                elif platform == 1:
                    text = raw.decode("mac_roman", errors="ignore")
                else:
                    continue
                english = platform == 1 or language in (0, 0x409)
                if text.strip() and (name_id not in names or english):
                    names[name_id] = text.strip()[:80]
            return names.get(16) or names.get(1)
    except (struct.error, IndexError, ValueError):
        return None
    return None


def project_fonts(folder: Path) -> list[dict]:
    """Font files in a project's assets with the family name to use."""
    assets = folder / "assets"
    fonts = []
    if not assets.is_dir() or assets.is_symlink():
        return fonts
    for current, folders, files in os.walk(assets):
        folders[:] = sorted(name for name in folders if not name.startswith("."))
        for name in sorted(files):
            path = Path(current) / name
            if path.suffix.lower() not in FONT_SUFFIXES or path.is_symlink():
                continue
            family = font_family(path)
            if family and FONT.fullmatch(family):
                fonts.append(
                    {"family": family, "path": path.relative_to(folder).as_posix()}
                )
            if len(fonts) >= MAX_FONT_FILES:
                return fonts
    return fonts
