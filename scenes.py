"""Read Manim scripts without executing them: scene discovery and formatting."""

from __future__ import annotations

import ast
import io
import re
import tokenize


class ScriptError(ValueError):
    """A script that cannot be read at all."""


def analyze(source: str) -> dict:
    """Discover scenes without importing or executing pasted code."""
    source = source.strip()
    if source.startswith("```"):
        lines = source.splitlines()
        if lines[-1].strip() == "```":
            source = "\n".join(lines[1:-1])
    try:
        tree = ast.parse(source)
    except UnicodeEncodeError as error:
        raise ScriptError(
            "The script contains invalid Unicode text. Paste or import it again."
        ) from error
    except RecursionError:
        return {
            "source": source,
            "scenes": [],
            "error": "This script contains an expression that is too deeply nested. Simplify it before rendering.",
        }
    except SyntaxError as error:
        return {
            "source": source,
            "scenes": [],
            "error": f"Line {error.lineno}: {error.msg}",
            "line": error.lineno,
        }
    bases = {
        "Scene",
        "MovingCameraScene",
        "ThreeDScene",
        "ZoomedScene",
        "VectorScene",
        "LinearTransformationScene",
    }
    modules = {"manim"}
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "manim":
            bases.update(
                alias.asname or alias.name
                for alias in node.names
                if alias.name in bases
            )
        elif isinstance(node, ast.Import):
            modules.update(
                alias.asname or alias.name
                for alias in node.names
                if alias.name == "manim"
            )
    scenes = []
    pending = [node for node in tree.body if isinstance(node, ast.ClassDef)]
    while pending:
        found = []
        for node in pending:
            if any(
                (isinstance(base, ast.Name) and base.id in bases)
                or (
                    isinstance(base, ast.Attribute)
                    and isinstance(base.value, ast.Name)
                    and base.value.id in modules
                    and base.attr in bases
                )
                for base in node.bases
            ):
                scenes.append(node.name)
                bases.add(node.name)
                found.append(node)
        if not found:
            break
        pending = [node for node in pending if node not in found]
    return {
        "source": source,
        "scenes": scenes,
        "error": None
        if scenes
        else "No Manim scene found. Include a class such as MainScene(Scene).",
        "needs_latex": any(
            isinstance(node, ast.Name)
            and node.id in {"Tex", "MathTex"}
            or isinstance(node, ast.Attribute)
            and node.attr in {"Tex", "MathTex"}
            for node in ast.walk(tree)
        ),
        "needs_typst": any(
            isinstance(node, ast.Name)
            and node.id in {"Typst", "MathTypst"}
            or isinstance(node, ast.Attribute)
            and node.attr in {"Typst", "MathTypst"}
            for node in ast.walk(tree)
        ),
    }


def format_source(source: str) -> str:
    """Normalize code whitespace while preserving every string literal byte."""
    if source.startswith("```") and source.rstrip().endswith("```"):
        source = "\n".join(source.splitlines()[1:-1])
    # Parse first: an unfinished literal must never be transformed speculatively.
    original_tree = ast.parse(source)
    protected = set()
    indentation = {}
    depth = 0
    new_statement = True
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
        if token.type == tokenize.INDENT:
            depth += 1
        elif token.type == tokenize.DEDENT:
            depth -= 1
        elif token.type == tokenize.NEWLINE:
            new_statement = True
        elif (
            token.type
            not in {
                tokenize.NL,
                tokenize.COMMENT,
                tokenize.ENCODING,
                tokenize.ENDMARKER,
            }
            and new_statement
        ):
            indentation[token.start[0]] = depth * 4
            new_statement = False
        if token.type == tokenize.STRING or tokenize.tok_name[token.type].startswith(
            "FSTRING"
        ):
            protected.update(range(token.start[0], token.end[0] + 1))
    lines = source.splitlines(keepends=True)
    for index, line in enumerate(lines, 1):
        if index not in protected:
            ending = "\n" if line.endswith("\n") else ""
            content = line.rstrip("\r\n")
            indent = re.match(r"[ \t]*", content)[0]
            prefix = " " * indentation[index] if index in indentation else indent
            lines[index - 1] = prefix + content[len(indent) :].rstrip() + ending
    result = "".join(lines)
    try:
        if ast.dump(ast.parse(result)) == ast.dump(original_tree):
            return result
    except SyntaxError:
        pass
    # Unusual layouts containing protected literals stay byte-for-byte intact.
    return source
