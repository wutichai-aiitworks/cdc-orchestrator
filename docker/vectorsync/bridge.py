"""The only module that imports pydebeziumai (ADR 0018).

pydebeziumai 0.1.0 is a first release: its documented `LiveContext` quickstart does not exist and
`SyncManager` dead-letters events on store outages and then returns normally. We therefore use
only its transformation layer (event model, id strategies, projection, document builder) and its
PGVector adapter, and run delivery ourselves (engine.py). When the package changes, this file is
the one to touch — keep every pydebeziumai name behind the functions below.
"""

from __future__ import annotations

import json
import string
from dataclasses import dataclass
from typing import Any

import pydebeziumai
from pydebeziumai.adapters.pgvector import PGVectorAdapter
from pydebeziumai.models.event import DebeziumEventModel
from pydebeziumai.transformation import (
    CompositeIdStrategy,
    DocumentBuilder,
    ProjectionPolicy,
    TablePkIdStrategy,
    TableProjectionPolicy,
)

VERSION = getattr(pydebeziumai, "__version__", "0.1.0")
OPS = {"c", "u", "d", "r"}


class DataError(Exception):
    """The record itself cannot be turned into a document (dead-letter it, commit, move on)."""


@dataclass
class Change:
    """One decoded Kafka record as pydebeziumai sees it, plus where it came from."""

    event: Any  # DebeziumEventModel
    doc_id: str
    op: str
    topic: str
    partition: int
    offset: int


def destination_for(topic: str) -> str:
    """pydebeziumai reads schema/table from the last two dotted segments of the destination.
    Our topics are `<prefix>.<schema>.<table>`, which already fits; a two-segment JDBC topic
    (`<prefix>.<table>`) gets an empty schema."""
    return topic


def to_event(topic: str, partition: int, key: Any, value: Any) -> Any | None:
    """Kafka record (already decoded to Python) → DebeziumEventModel, or None to skip.

    Two value shapes reach us: the Debezium envelope (log-based capture: before/after/op/source)
    and a flat row (incremental JDBC capture, ADR 0004), which we treat as a snapshot read ("r").
    Tombstones and schema-change events are skipped.
    """
    if value is None:
        return None  # tombstone after a delete
    if not isinstance(value, dict):
        raise DataError(f"value is {type(value).__name__}, expected a record")
    if "payload" in value and isinstance(value["payload"], dict) and "schema" in value:
        value = value["payload"]  # JSON converter with schemas.enable=true
    if "op" in value:
        op = value.get("op")
        if op not in OPS:
            if op == "t":
                return None  # truncate: nothing per row to sync
            raise DataError(f"unsupported op {op!r}")
        payload = {
            "before": value.get("before"),
            "after": value.get("after"),
            "op": op,
            "ts_ms": value.get("ts_ms"),
            "source": value.get("source"),
        }
    elif "ddl" in value or "tableChanges" in value:
        return None  # schema history / DDL event
    else:
        payload = {"before": None, "after": value, "op": "r", "ts_ms": None}
    key_str = json.dumps(key, sort_keys=True, default=str) if isinstance(key, dict) else (None if key is None else str(key))
    try:
        return DebeziumEventModel(
            destination=destination_for(topic),
            partition=partition,
            key=key_str,
            payload=payload,
        )
    except Exception as exc:  # pydantic validation
        raise DataError(f"not a change event: {exc}") from exc


def _table_policy(cfg: dict[str, Any] | None) -> TableProjectionPolicy:
    cfg = cfg or {}
    template = (cfg.get("content_template") or "").strip() or None
    doc_col = (cfg.get("document") or {}).get("column")
    include = [f for f in (cfg.get("include_fields") or []) if f and f != doc_col] or None
    metadata = cfg.get("metadata_fields")
    return TableProjectionPolicy(
        include_fields=include,
        # The Docling column holds file bytes: never embedded as text, never stored as metadata.
        exclude_fields=[f for f in (cfg.get("exclude_fields") or []) if f] + ([cfg["document"]["column"]] if (cfg.get("document") or {}).get("column") else []),
        metadata_fields=[f for f in metadata if f] if isinstance(metadata, list) else None,
        content_template=template,
    )


def template_fields(template: str | None) -> list[str]:
    """`$name` / `${name}` placeholders in a content template (`$$` is a literal dollar)."""
    if not template:
        return []
    return sorted(set(string.Template(template).get_identifiers()))


def check_template(template: str | None) -> str | None:
    """A template error message, or None. pydebeziumai only logs a bad template and embeds the
    text '[template error: …]', so templates are checked before a sync is accepted."""
    if not template:
        return None
    t = string.Template(template)
    if not t.is_valid():
        return "Invalid template: use $column or ${column}; write $$ for a literal dollar sign."
    return None


def make_builder(spec: dict[str, Any]) -> Any:
    proj = spec.get("projection") or {}
    tables = proj.get("tables") or {}
    policy = ProjectionPolicy(
        default=_table_policy(proj.get("default")),
        overrides={name: _table_policy(cfg) for name, cfg in tables.items()},
    )
    ids = spec.get("id_strategy") or {}
    pk_fields = [f for f in (ids.get("pk_fields") or []) if f] or None
    if ids.get("kind") == "composite":
        if not pk_fields:
            raise ValueError("id_strategy composite needs pk_fields")
        strategy = CompositeIdStrategy(pk_fields)
    else:
        strategy = TablePkIdStrategy(pk_fields)
    extra = {"_sync": spec.get("name") or spec.get("id") or ""}
    return DocumentBuilder(id_strategy=strategy, projection_policy=policy, extra_metadata=extra)


def doc_id(builder: Any, event: Any) -> str:
    try:
        return builder.id_strategy.generate(event)
    except Exception as exc:
        raise DataError(f"no document id: {exc}") from exc


def build(builder: Any, event: Any, soft_delete: bool) -> Any:
    """LangChain Document for an upsert (or soft delete); raises DataError for bad rows."""
    try:
        result = builder.build(event, allow_soft_delete=soft_delete)
    except Exception as exc:
        raise DataError(f"cannot build document: {exc}") from exc
    if result.document is None:
        raise DataError("empty document")
    content = result.document.page_content or ""
    if "[template error:" in content:
        raise DataError(content[content.index("[template error:"):])
    return result.document


def make_adapter(url: str, collection: str, embeddings: Any, reset: bool = False) -> Any:
    # An Engine instead of a URL: pool_pre_ping drops connections a Postgres restart killed
    # (an idle sync otherwise fails its next search/drop with AdminShutdown).
    from sqlalchemy import create_engine

    engine = create_engine(url, pool_pre_ping=True, pool_recycle=1800)
    return PGVectorAdapter(
        connection_string=engine,  # type: ignore[arg-type] — langchain_postgres accepts an Engine
        collection_name=collection,
        embeddings=embeddings,
        pre_delete_collection=reset,
    )


def search(adapter: Any, query: str, k: int, metadata_filter: dict[str, Any] | None) -> list[dict[str, Any]]:
    store = adapter.store
    hits = store.similarity_search_with_score(query, k=k, filter=metadata_filter or None)
    out = []
    for doc, score in hits:
        out.append(
            {
                "id": doc.id or doc.metadata.get("_doc_id"),
                "score": round(1.0 - float(score), 4),  # cosine distance → similarity
                "content": doc.page_content,
                "metadata": doc.metadata,
            }
        )
    return out
