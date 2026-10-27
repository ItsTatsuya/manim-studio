"""The user's own watermark (a logo or a line of text) on finished videos.

Settings come from three places, most specific first: a PDF-batch video
workspace, its project, and the global default in `DATA_ROOT/.studio/`.
Uploaded logos are rasterised once to PNG and kept, content-addressed, in
`DATA_ROOT/.studio/watermark/logos/`, so queued renders keep the logo they were
started with even if the settings change later.

The render worker applies the watermark as the last step, after narration is
muxed, by re-encoding the video with the same FFmpeg libraries (PyAV) that Manim
and narration use and copying the audio untouched. The animation cache stores the
video before this step, so a new watermark never forces Manim to render again.
"""

from __future__ import annotations

import hashlib
import io
import json
import math
from pathlib import Path
import re
import uuid
from typing import Literal

from pydantic import BaseModel, Field, ValidationError, model_validator

POSITIONS = (
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
    "center",
    "bottom-center",
)
LOGO_SUFFIXES = {".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp", ".svg"}
MAX_LOGO_BYTES = 10 * 1024**2
MAX_SOURCE_PIXELS = 40_000_000
# Stored logos are big enough for a 4K video at full width.
MAX_LOGO_SIDE = 2048
LOGO_ID = re.compile(r"[0-9a-f]{32}")
# Re-encoding is a second generation, so keep it visually lossless.
VIDEO_CRF = "18"
FONT_CANDIDATES = (
    "segoeuib.ttf",
    "seguisb.ttf",
    "arialbd.ttf",
    "DejaVuSans-Bold.ttf",
    "segoeui.ttf",
    "arial.ttf",
    "DejaVuSans.ttf",
)


class WatermarkError(Exception):
    """A user-facing watermark problem."""


class Settings(BaseModel):
    type: Literal["none", "image", "text"] = "none"
    logo: str | None = Field(default=None, pattern=r"^[0-9a-f]{32}$")
    logo_name: str = Field(default="", max_length=200)
    text: str = Field(default="", max_length=200)
    # Pixels on a 1080-pixel-tall video; other heights scale proportionally.
    font_size: int = Field(default=40, ge=8, le=400)
    color: str = Field(default="#ffffff", pattern=r"^#[0-9a-fA-F]{6}$")
    position: Literal[
        "top-left",
        "top-right",
        "bottom-left",
        "bottom-right",
        "center",
        "bottom-center",
    ] = "bottom-right"
    # Percent of the video width.
    margin: float = Field(default=3.0, ge=0, le=20, allow_inf_nan=False)
    scale: float = Field(default=15.0, ge=1, le=100, allow_inf_nan=False)
    opacity: float = Field(default=0.8, ge=0, le=1, allow_inf_nan=False)

    @model_validator(mode="after")
    def complete(self):
        if self.type == "image" and not self.logo:
            raise ValueError("Choose a logo image for the watermark.")
        if self.type == "text" and not self.text.strip():
            raise ValueError("Enter the watermark text.")
        if any(ord(character) < 32 for character in self.text):
            raise ValueError("Keep the watermark text on one line.")
        return self


DEFAULT = Settings().model_dump()


def clean(value) -> dict | None:
    """Stored settings, or None when missing or damaged."""
    if not isinstance(value, dict):
        return None
    try:
        return Settings.model_validate(value).model_dump()
    except (ValidationError, ValueError, TypeError, RecursionError):
        return None


# ------------------------------------------------------------------ storage


def _studio(root: Path) -> Path:
    return root / ".studio"


def default_path(root: Path) -> Path:
    return _studio(root) / "watermark.json"


def logo_dir(root: Path) -> Path:
    return _studio(root) / "watermark" / "logos"


def load_default(root: Path) -> dict:
    try:
        saved = json.loads(default_path(root).read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError):
        return dict(DEFAULT)
    return clean(saved) or dict(DEFAULT)


def save_default(root: Path, settings: dict) -> dict:
    settings = Settings.model_validate(settings).model_dump()
    path = default_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f"watermark.{uuid.uuid4().hex}.tmp")
    try:
        temporary.write_text(json.dumps(settings, indent=2), encoding="utf-8")
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)
    return settings


def logo_path(root: Path, logo: str) -> Path:
    if not isinstance(logo, str) or not LOGO_ID.fullmatch(logo):
        raise KeyError(logo)
    return logo_dir(root) / f"{logo}.png"


def effective(root: Path, own: dict | None, parent: dict | None = None):
    """The settings a render uses, and where they came from."""
    own, parent = clean(own), clean(parent)
    if own is not None:
        return own, "own"
    if parent is not None:
        return parent, "project"
    return load_default(root), "default"


def render_spec(root: Path, settings: dict | None) -> dict | None:
    """What the render worker needs, or None for no watermark.

    Raises WatermarkError when the chosen logo is no longer available.
    """
    settings = clean(settings)
    if settings is None or settings["type"] == "none":
        return None
    if settings["type"] == "text":
        if settings["opacity"] <= 0:
            return None
        return {**settings, "logo_file": None}
    path = logo_path(root, settings["logo"])
    if not path.is_file():
        raise WatermarkError(
            "The watermark logo is missing. Choose the logo again in watermark settings."
        )
    if settings["opacity"] <= 0:
        return None
    return {**settings, "logo_file": str(path)}


# ------------------------------------------------------------------ logos


def _safe_label(filename: str) -> str:
    name = Path((filename or "").replace("\\", "/")).name
    name = re.sub(r"[\x00-\x1f<>:\"/\\|?*]+", "", name).strip()
    return name[:200] or "logo"


def rasterise_svg(data: bytes, side: int = 1024):
    """Draw an SVG's shapes with Cairo (the library Manim renders with)."""
    import cairo
    import svgelements as svg
    from PIL import Image

    try:
        document = svg.SVG.parse(io.BytesIO(data), reify=True)
    except Exception as error:  # svgelements raises a variety of parse errors.
        raise WatermarkError(
            "This SVG couldn't be read. Export the logo as PNG and try again."
        ) from error
    shapes = [
        element
        for element in document.elements()
        if isinstance(element, svg.Shape)
        and not isinstance(element, svg.SVG)
        and element.values.get("visibility") not in {"hidden", "collapse"}
    ]
    paths = []
    for element in shapes:
        try:
            path = abs(svg.Path(element))
        except Exception:
            continue
        if len(path):
            paths.append((element, path))
    boxes = [path.bbox() for _, path in paths]
    boxes = [box for box in boxes if box is not None]
    viewbox = getattr(document, "viewbox", None)
    if viewbox is not None and viewbox.width and viewbox.height:
        left, top = float(viewbox.x), float(viewbox.y)
        width, height = float(viewbox.width), float(viewbox.height)
    elif boxes:
        left = min(box[0] for box in boxes)
        top = min(box[1] for box in boxes)
        width = max(box[2] for box in boxes) - left
        height = max(box[3] for box in boxes) - top
    else:
        width = height = 0
    if (
        not paths
        or not (width > 0 and height > 0)
        or not all(math.isfinite(value) for value in (width, height))
    ):
        raise WatermarkError(
            "This SVG has no shapes Studio can draw. Export the logo as PNG and try again."
        )
    factor = side / max(width, height)
    pixels = (max(1, round(width * factor)), max(1, round(height * factor)))
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, *pixels)
    context = cairo.Context(surface)
    context.scale(factor, factor)
    context.translate(-left, -top)
    for element, path in paths:
        context.new_path()
        for segment in path:
            if isinstance(segment, svg.Move):
                context.move_to(segment.end.x, segment.end.y)
            elif isinstance(segment, svg.Close):
                context.close_path()
            elif isinstance(segment, svg.Line):
                context.line_to(segment.end.x, segment.end.y)
            elif isinstance(segment, svg.CubicBezier):
                context.curve_to(
                    segment.control1.x,
                    segment.control1.y,
                    segment.control2.x,
                    segment.control2.y,
                    segment.end.x,
                    segment.end.y,
                )
            elif isinstance(segment, svg.QuadraticBezier):
                start, control, end = segment.start, segment.control, segment.end
                context.curve_to(
                    start.x + 2 / 3 * (control.x - start.x),
                    start.y + 2 / 3 * (control.y - start.y),
                    end.x + 2 / 3 * (control.x - end.x),
                    end.y + 2 / 3 * (control.y - end.y),
                    end.x,
                    end.y,
                )
            elif isinstance(segment, svg.Arc):
                for curve in segment.as_cubic_curves():
                    context.curve_to(
                        curve.control1.x,
                        curve.control1.y,
                        curve.control2.x,
                        curve.control2.y,
                        curve.end.x,
                        curve.end.y,
                    )
        fill, stroke = element.fill, element.stroke
        context.set_fill_rule(
            cairo.FILL_RULE_EVEN_ODD
            if element.values.get("fill-rule") == "evenodd"
            else cairo.FILL_RULE_WINDING
        )
        if fill is not None and fill.value is not None and fill.alpha:
            context.set_source_rgba(
                fill.red / 255, fill.green / 255, fill.blue / 255, fill.alpha / 255
            )
            context.fill_preserve()
        width = float(getattr(element, "implicit_stroke_width", 0) or 0)
        if stroke is not None and stroke.value is not None and stroke.alpha and width:
            context.save()
            context.identity_matrix()
            context.set_line_width(width * factor)
            context.set_source_rgba(
                stroke.red / 255,
                stroke.green / 255,
                stroke.blue / 255,
                stroke.alpha / 255,
            )
            context.stroke_preserve()
            context.restore()
    surface.flush()
    # Cairo stores premultiplied BGRA; Pillow converts it to straight RGBA.
    image = Image.frombuffer(
        "RGBa", pixels, bytes(surface.get_data()), "raw", "BGRa", surface.get_stride()
    ).convert("RGBA")
    if image.getbbox() is None:
        raise WatermarkError(
            "This SVG has no visible shapes Studio can draw. Export the logo as PNG and try again."
        )
    return image


def rasterise(data: bytes, filename: str):
    """Decode an uploaded logo into an RGBA Pillow image."""
    from PIL import Image, ImageOps, UnidentifiedImageError

    suffix = Path(filename or "").suffix.lower()
    if suffix not in LOGO_SUFFIXES:
        raise WatermarkError("Choose a PNG, JPG, WebP, GIF, BMP or SVG logo.")
    if len(data) > MAX_LOGO_BYTES:
        raise WatermarkError("Logos must be smaller than 10 MB.")
    if not data:
        raise WatermarkError("This logo file is empty.")
    if suffix == ".svg":
        return rasterise_svg(data)
    try:
        with Image.open(io.BytesIO(data)) as source:
            if source.width * source.height > MAX_SOURCE_PIXELS:
                raise WatermarkError(
                    "This logo is too large. Use an image under 40 megapixels."
                )
            source.draft("RGB", (MAX_LOGO_SIDE, MAX_LOGO_SIDE))
            image = ImageOps.exif_transpose(source).convert("RGBA")
    except WatermarkError:
        raise
    except (UnidentifiedImageError, OSError, ValueError, Image.DecompressionBombError):
        raise WatermarkError(
            "This logo couldn't be read. Export it as PNG and try again."
        ) from None
    image.thumbnail((MAX_LOGO_SIDE, MAX_LOGO_SIDE), Image.Resampling.LANCZOS)
    return image


def store_logo(root: Path, data: bytes, filename: str) -> dict:
    """Rasterise and keep a logo; identical logos share one file."""
    image = rasterise(data, filename)
    bbox = image.getbbox()
    if bbox is None:
        raise WatermarkError("This logo is fully transparent.")
    image = image.crop(bbox)
    output = io.BytesIO()
    image.save(output, format="PNG", optimize=True)
    png = output.getvalue()
    logo = hashlib.sha256(png).hexdigest()[:32]
    directory = logo_dir(root)
    directory.mkdir(parents=True, exist_ok=True)
    target = directory / f"{logo}.png"
    if not target.is_file():
        temporary = directory / f".{uuid.uuid4().hex}.tmp"
        try:
            temporary.write_bytes(png)
            temporary.replace(target)
        finally:
            temporary.unlink(missing_ok=True)
    return {
        "logo": logo,
        "name": _safe_label(filename),
        "width": image.width,
        "height": image.height,
    }


# ------------------------------------------------------------------ drawing


def _font(size: int):
    from PIL import ImageFont

    for name in FONT_CANDIDATES:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default(size)


def text_image(text: str, size: int, color: str):
    """Draw text into a tight transparent PNG (no FFmpeg font configuration)."""
    from PIL import Image, ImageDraw

    font = _font(max(1, size))
    probe = ImageDraw.Draw(Image.new("RGBA", (1, 1)))
    left, top, right, bottom = probe.textbbox((0, 0), text, font=font)
    pad = max(1, size // 10)
    image = Image.new(
        "RGBA", (max(1, right - left + 2 * pad), max(1, bottom - top + 2 * pad))
    )
    rgb = tuple(int(color[index : index + 2], 16) for index in (1, 3, 5))
    ImageDraw.Draw(image).text(
        (pad - left, pad - top), text, font=font, fill=(*rgb, 255)
    )
    return image


def overlay(spec: dict, width: int, height: int):
    """The watermark image at its final size and opacity, and its position."""
    from PIL import Image

    if spec["type"] == "text":
        image = text_image(
            spec["text"].strip(),
            max(1, round(spec["font_size"] * height / 1080)),
            spec["color"],
        )
    else:
        try:
            with Image.open(spec["logo_file"]) as source:
                image = source.convert("RGBA")
        except OSError as error:
            raise WatermarkError(
                "The watermark logo couldn't be read. Choose it again in watermark settings."
            ) from error
        target = max(1, round(width * spec["scale"] / 100))
        image = image.resize(
            (target, max(1, round(image.height * target / image.width))),
            Image.Resampling.LANCZOS,
        )
    margin = round(width * spec["margin"] / 100)
    # Never let the watermark spill past the frame.
    room_w = max(1, width - 2 * margin)
    room_h = max(1, height - 2 * margin)
    if image.width > room_w or image.height > room_h:
        ratio = min(room_w / image.width, room_h / image.height)
        image = image.resize(
            (max(1, int(image.width * ratio)), max(1, int(image.height * ratio))),
            Image.Resampling.LANCZOS,
        )
    if spec["opacity"] < 1:
        alpha = image.getchannel("A").point(
            lambda value: round(value * spec["opacity"])
        )
        image.putalpha(alpha)
    return image, placement(spec["position"], width, height, image.size, margin)


def placement(position: str, width: int, height: int, size, margin: int):
    w, h = size
    x = {
        "top-left": margin,
        "bottom-left": margin,
        "top-right": width - w - margin,
        "bottom-right": width - w - margin,
    }.get(position, (width - w) // 2)
    y = {
        "top-left": margin,
        "top-right": margin,
        "center": (height - h) // 2,
    }.get(position, height - h - margin)
    return max(0, x), max(0, y)


# ------------------------------------------------------------------ video


def apply(spec: dict, video: Path, output: Path, report=lambda message: None):
    """Write `output`: `video` with the watermark, audio copied untouched."""
    import av
    import av.filter
    from fractions import Fraction
    import numpy

    with av.open(str(video)) as source:
        if not source.streams.video:
            raise WatermarkError("The rendered file doesn't contain a video stream.")
        in_video = source.streams.video[0]
        in_audio = source.streams.audio[0] if source.streams.audio else None
        width, height = in_video.codec_context.width, in_video.codec_context.height
        image, (x, y) = overlay(spec, width, height)
        rate = in_video.average_rate or in_video.guessed_rate or Fraction(30)
        total = in_video.frames or 0
        if not total and in_video.duration is not None and in_video.time_base:
            total = round(float(in_video.duration * in_video.time_base) * rate)
        time_base = in_video.time_base or Fraction(1, 1000)

        with av.open(str(output), "w", format="mp4") as target:
            out_video = target.add_stream("libx264", rate=rate)
            out_video.width, out_video.height = width, height
            out_video.pix_fmt = "yuv420p"
            out_video.time_base = time_base
            out_video.options = {"crf": VIDEO_CRF}
            out_audio = (
                target.add_stream_from_template(in_audio)
                if in_audio is not None
                else None
            )

            graph = av.filter.Graph()
            main = graph.add_buffer(template=in_video)
            logo = graph.add(
                "buffer",
                width=str(image.width),
                height=str(image.height),
                pix_fmt="rgba",
                time_base=f"{time_base.numerator}/{time_base.denominator}",
                pixel_aspect="1/1",
            )
            mixer = graph.add(
                "overlay", x=str(x), y=str(y), format="auto", eof_action="repeat"
            )
            convert = graph.add("format", "yuv420p")
            sink = graph.add("buffersink")
            main.link_to(mixer, 0, 0)
            logo.link_to(mixer, 0, 1)
            mixer.link_to(convert)
            convert.link_to(sink)
            graph.configure()
            # from_image would drop the alpha channel; keep it as RGBA.
            still = av.VideoFrame.from_ndarray(
                numpy.ascontiguousarray(numpy.asarray(image)), format="rgba"
            )
            started = False
            done, shown = 0, -1

            def drain():
                nonlocal done, shown
                while True:
                    try:
                        frame = sink.pull()
                    except (av.error.BlockingIOError, av.error.EOFError):
                        return
                    for packet in out_video.encode(frame):
                        target.mux(packet)
                    done += 1
                    if total:
                        percent = min(99, done * 100 // total)
                        if percent >= shown + 2:
                            shown = percent
                            report(f"Adding your watermark · {percent}%")

            report("Adding your watermark…")
            streams = [in_video] + ([in_audio] if in_audio is not None else [])
            for packet in source.demux(*streams):
                if packet.stream is in_video:
                    for frame in packet.decode():
                        if not started:
                            # The logo starts with the first frame and then holds.
                            still.pts = frame.pts or 0
                            still.time_base = time_base
                            logo.push(still)
                            logo.push(None)
                            started = True
                        main.push(frame)
                        drain()
                elif packet.dts is not None and out_audio is not None:
                    packet.stream = out_audio
                    target.mux(packet)
            if not started:
                raise WatermarkError("The rendered video has no frames to watermark.")
            main.push(None)
            drain()
            for packet in out_video.encode(None):
                target.mux(packet)
    report("Adding your watermark · 100%")


def rendered_video(media_dir: Path) -> Path | None:
    """The finished movie the worker would hand to Studio (newest wins)."""
    videos = [
        path
        for path in Path(media_dir).rglob("video.mp4")
        if "partial_movie_files" not in path.parts
    ]
    return max(videos, key=lambda path: path.stat().st_mtime_ns) if videos else None


def finish_render(spec_file: str | Path, report=print):
    """Called by the render worker once the final (narrated) video exists."""
    spec = json.loads(Path(spec_file).read_text(encoding="utf-8"))
    video = rendered_video(Path(spec["media_dir"]))
    if video is None:
        return
    staged = video.with_name("watermarked.mp4")
    try:
        apply(spec, video, staged, report=report)
        staged.replace(video)
    except WatermarkError:
        raise
    except Exception as error:
        raise WatermarkError(
            f"Adding the watermark failed ({type(error).__name__}: {error})."
        ) from error
    finally:
        staged.unlink(missing_ok=True)
