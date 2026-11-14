"""Docling: file bytes in a column → structure-aware text chunks (ADR 0018 amendment).

Only in the `cdc-vectorsync` image built with WITH_DOCLING=true (VECTORSYNC_DOCLING=true); the
default image has no Docling and `AVAILABLE` is False. Models (layout, tableformer) are baked into
the image at DOCLING_ARTIFACTS_PATH and Hugging Face runs offline, so conversion needs no network.

PDF, DOCX, PPTX, XLSX, HTML, Markdown, CSV and images are recognised from a filename column or the
file's magic bytes. OCR is off (scanned PDFs yield little text). Chunks come from Docling's
HierarchicalChunker (sections and tables, each prefixed with its heading path), merged up to
`max_chars` and split further when one is longer.
"""

from __future__ import annotations

import base64
import binascii
import io
import os
import threading
import zipfile
from typing import Any

from chunking import merge_small, split_text

try:  # optional dependency
    from docling.datamodel.base_models import DocumentStream, InputFormat
    from docling.datamodel.pipeline_options import PdfPipelineOptions
    from docling.document_converter import DocumentConverter, PdfFormatOption
    from docling_core.transforms.chunker import HierarchicalChunker

    AVAILABLE = True
except ImportError:  # pragma: no cover - default image
    AVAILABLE = False

MAX_MB = float(os.environ.get("VECTORSYNC_DOCLING_MAX_MB", "50"))
MAX_PAGES = int(os.environ.get("VECTORSYNC_DOCLING_MAX_PAGES", "500"))
FORMATS = ("pdf", "docx", "pptx", "xlsx", "html", "md", "csv", "png", "jpg", "tiff")


class DocumentError(Exception):
    """The column does not hold a readable document (dead-letter the row)."""


def version() -> str | None:
    if not AVAILABLE:
        return None
    from importlib.metadata import version as v

    return v("docling")


def to_bytes(value: Any) -> bytes:
    """Column value → file bytes. Avro/JDBC bytes come as bytes; JSON converters as base64;
    Postgres text output of bytea as \\x<hex>."""
    if isinstance(value, (bytes, bytearray, memoryview)):
        return bytes(value)
    if isinstance(value, str):
        s = value.strip()
        if s.startswith("\\x"):
            try:
                return bytes.fromhex(s[2:])
            except ValueError:
                pass
        try:
            return base64.b64decode(s, validate=True)
        except (binascii.Error, ValueError):
            raise DocumentError("the document column holds text, not file bytes (expected bytea/BLOB or base64)") from None
    if value is None:
        raise DocumentError("the document column is empty")
    raise DocumentError(f"the document column holds {type(value).__name__}, not file bytes")


def sniff(data: bytes, filename: str | None) -> str:
    """A filename with the right extension (Docling picks the format from it)."""
    if filename:
        ext = filename.rsplit(".", 1)[-1].lower() if "." in filename else ""
        if ext in FORMATS or ext in ("htm", "jpeg", "tif", "markdown"):
            return filename
    head = data[:8]
    if head.startswith(b"%PDF"):
        return "document.pdf"
    if head.startswith(b"PK"):
        try:
            names = zipfile.ZipFile(io.BytesIO(data)).namelist()
        except zipfile.BadZipFile:
            raise DocumentError("the file looks like a ZIP archive but cannot be opened") from None
        if any(n.startswith("word/") for n in names):
            return "document.docx"
        if any(n.startswith("ppt/") for n in names):
            return "document.pptx"
        if any(n.startswith("xl/") for n in names):
            return "document.xlsx"
        raise DocumentError("ZIP file that is not DOCX/PPTX/XLSX")
    if head.startswith(b"\x89PNG"):
        return "image.png"
    if head[:3] == b"\xff\xd8\xff":
        return "image.jpg"
    if head[:4] in (b"II*\x00", b"MM\x00*"):
        return "image.tiff"
    text = data[:512].lstrip().lower()
    if text.startswith((b"<!doctype html", b"<html")):
        return "document.html"
    if text.startswith(b"#"):
        return "document.md"
    raise DocumentError("unrecognised file type (supported: PDF, DOCX, PPTX, XLSX, HTML, Markdown, CSV, PNG/JPEG/TIFF)")


_lock = threading.Lock()
_converter: Any = None


def _get_converter() -> Any:
    global _converter
    if _converter is None:
        opts = PdfPipelineOptions(
            do_ocr=False,
            do_table_structure=True,
            enable_remote_services=False,
            artifacts_path=os.environ.get("DOCLING_ARTIFACTS_PATH") or None,
        )
        _converter = DocumentConverter(format_options={InputFormat.PDF: PdfFormatOption(pipeline_options=opts)})
    return _converter


def to_chunks(value: Any, filename: str | None, max_chars: int, overlap: int) -> tuple[list[str], dict[str, Any]]:
    """Convert one document and chunk it. Raises DocumentError for bad files."""
    if not AVAILABLE:
        raise DocumentError("Docling is not in this image (set VECTORSYNC_DOCLING=true and redeploy)")
    data = to_bytes(value)
    if not data:
        raise DocumentError("the document column is empty")
    if len(data) > MAX_MB * 1024 * 1024:
        raise DocumentError(f"file is {len(data) / 1048576:.1f} MB, over VECTORSYNC_DOCLING_MAX_MB={MAX_MB:g}")
    name = sniff(data, filename)
    # One conversion at a time: the models are large and use every core already.
    with _lock:
        try:
            res = _get_converter().convert(
                DocumentStream(name=name, stream=io.BytesIO(data)),
                max_num_pages=MAX_PAGES,
                max_file_size=int(MAX_MB * 1024 * 1024),
            )
        except Exception as exc:  # noqa: BLE001 — corrupt / unsupported file
            raise DocumentError(f"Docling could not read the {name.rsplit('.', 1)[-1].upper()} file: {str(exc)[:200]}") from exc
        doc = res.document
        chunker = HierarchicalChunker()
        pieces = [chunker.contextualize(c) for c in chunker.chunk(doc)]
    if not pieces:
        text = doc.export_to_markdown().strip()
        pieces = [text] if text else []
    if not pieces:
        raise DocumentError("no text found in the document (scanned pages need OCR, which is off)")
    chunks: list[str] = []
    for merged in merge_small(pieces, max_chars):
        chunks.extend(split_text(merged, max_chars, overlap) if len(merged) > max_chars else [merged])
    pages = len(getattr(doc, "pages", {}) or {})
    return chunks, {"_doc_format": name.rsplit(".", 1)[-1].lower(), "_doc_pages": pages, "_doc_bytes": len(data)}
