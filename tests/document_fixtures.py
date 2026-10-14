"""Valid, small reference documents for upload integration checks."""

from pathlib import Path

DIRECTORY = Path(__file__).parent / "fixtures" / "documents"
NAMES = (
    ("reference.pdf", "Lesson notes.pdf", "application/pdf"),
    (
        "reference.docx",
        "Résumé (100% #1).docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ),
    ("reference.txt", "Notes (#1).txt", "text/plain"),
    ("reference.md", "outline.md", "text/markdown"),
)


def documents():
    return [
        {"name": name, "mimeType": mime, "buffer": (DIRECTORY / source).read_bytes()}
        for source, name, mime in NAMES
    ]
