# Document upload verification

Verified on 14 October 2026 before packaging the first release, 1.0.0.

The fixtures are valid documents: a compressed three-page PDF, a Word document
with headings and paragraphs, UTF-8 text, and Markdown. Their test content is
owned by this repository. No private documents or existing user projects are used.

| Check | Result |
| --- | --- |
| Attachment menu and multiple-file picker | Pass |
| Visible Windows document and media dialogs, selection and cancel | Pass with real native dialogs |
| Native Assets-panel picker, duplicate selection, and cancel | Pass |
| Drag-and-drop and concurrent uploads | Pass |
| PDF, DOCX, TXT, and Markdown byte integrity | Pass |
| Unicode, spaces, percent signs, and hash signs in filenames | Pass |
| Duplicate filenames without overwriting originals | Pass |
| Upload progress and preventing early sends | Pass |
| Removing a reference and cancelling the picker | Pass |
| Network failure and reselecting the same file | Pass |
| One failed Assets upload without dropping the rest of a batch | Pass after fix |
| Failed send retains the message and attachment, without duplicate chats | Pass after fix |
| Navigation during upload and persistence in the original project | Pass |
| Empty files, missing filenames, size limits, and interrupted stream cleanup | Pass |
| Codex and Claude subprocesses read all four files with matching SHA256 hashes | Pass with deterministic CLI fixtures |
| Live Codex reads the documents and generates the specified scene and narration | Pass |
| Render of that generated scene with real Kokoro narration | Pass |
| Live Claude document generation | Pass on retry after the account quota reset |

The full Python suite passes **163 tests**, and the browser suite passes **23
tests**, including real narration and rendering. Accessibility and layout pass
across 14 views, both themes, and three viewport sizes. Formatting and static
checks pass.

The initial live Claude request was quota-limited. Its error displayed correctly
and preserved the uploaded documents. After the 10:30pm IST reset, the live retry
read all four documents, generated the specified shapes, labels and narration,
included all four document markers, and passed the Studio scene check. This retry
completed at 17:02 UTC while release packaging was running.

Run the checks with:

```powershell
uv run python -m unittest discover -s tests -v
uv run python tests/ui_smoke.py -v
uv run python tests/react_audit.py
uv run python tests/native_file_picker.py
```

Set `MANIM_STUDIO_TEST_KOKORO` to the local Kokoro model directory to include the
real narration tests. Test fixtures live in `tests/fixtures/documents`.
The native picker check requires an interactive Windows desktop. It verifies actual
visible dialogs rather than intercepted HTML file-chooser events.
