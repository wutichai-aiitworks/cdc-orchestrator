"""Kafka record bytes → Python values for cdc-vectorsync.

Connect writes Confluent-framed Avro (magic 0 + 4-byte schema id + body). We resolve the writer
schema from the registry REST API (cached by id), decode with fastavro, then turn Debezium's
integer time types and VariableScaleDecimal into readable values — the text is what gets
embedded, so `created_at: 2026-11-14T08:00:00` beats `created_at: 1763107200000000`.
Anything that is not Confluent Avro is tried as JSON.
"""

from __future__ import annotations

import base64
import datetime as dt
import io
import json
import threading
import urllib.error
import urllib.request
from decimal import Decimal
from typing import Any

import fastavro

PRIMITIVES = {"null", "boolean", "int", "long", "float", "double", "bytes", "string"}
EPOCH = dt.datetime(1970, 1, 1)


class RegistryUnavailable(Exception):
    """Schema registry could not be reached — an infrastructure error (retry, don't commit)."""


class DecodeError(Exception):
    """The bytes are not a record we can read — a data error (dead-letter)."""


def _time_value(connect_name: str, v: Any) -> Any:
    if not isinstance(v, int) or isinstance(v, bool):
        return v
    try:
        if connect_name == "io.debezium.time.Date":
            return (EPOCH + dt.timedelta(days=v)).date().isoformat()
        if connect_name == "io.debezium.time.Timestamp":
            return (EPOCH + dt.timedelta(milliseconds=v)).isoformat()
        if connect_name == "io.debezium.time.MicroTimestamp":
            return (EPOCH + dt.timedelta(microseconds=v)).isoformat()
        if connect_name == "io.debezium.time.NanoTimestamp":
            return (EPOCH + dt.timedelta(microseconds=v // 1000)).isoformat()
        if connect_name == "io.debezium.time.Time":
            return (EPOCH + dt.timedelta(milliseconds=v)).time().isoformat()
        if connect_name == "io.debezium.time.MicroTime":
            return (EPOCH + dt.timedelta(microseconds=v)).time().isoformat()
        if connect_name == "io.debezium.time.NanoTime":
            return (EPOCH + dt.timedelta(microseconds=v // 1000)).time().isoformat()
    except (OverflowError, ValueError):
        return v
    return v


def _variable_decimal(v: Any) -> Any:
    if isinstance(v, dict) and "scale" in v and "value" in v:
        raw = v["value"]
        if isinstance(raw, str):
            raw = base64.b64decode(raw)
        if isinstance(raw, (bytes, bytearray)):
            unscaled = int.from_bytes(raw, "big", signed=True)
            return Decimal(unscaled).scaleb(-int(v["scale"]))
    return v


class Normalizer:
    """Walks a writer schema next to the decoded value (named types included)."""

    def __init__(self, schema: Any) -> None:
        self.named: dict[str, Any] = {}
        self._index(schema, "")

    def _index(self, s: Any, ns: str) -> None:
        if isinstance(s, list):
            for b in s:
                self._index(b, ns)
        elif isinstance(s, dict):
            t = s.get("type")
            if t in ("record", "enum", "fixed") and s.get("name"):
                space = s.get("namespace", ns)
                name = s["name"]
                full = name if "." in name or not space else f"{space}.{name}"
                self.named[full] = s
                self.named.setdefault(name, s)
                ns = space
            if t == "record":
                for f in s.get("fields", []):
                    self._index(f.get("type"), ns)
            elif t == "array":
                self._index(s.get("items"), ns)
            elif t == "map":
                self._index(s.get("values"), ns)
            elif isinstance(t, (dict, list)):
                self._index(t, ns)

    def norm(self, s: Any, v: Any) -> Any:
        if v is None:
            return None
        if isinstance(s, str):
            if s in PRIMITIVES:
                return v
            s = self.named.get(s, s)
            if isinstance(s, str):
                return v
        if isinstance(s, list):
            branches = [b for b in s if b != "null"]
            if len(branches) == 1:
                return self.norm(branches[0], v)
            for b in branches:  # pick a record branch whose fields match, else leave as is
                bs = self.named.get(b, b) if isinstance(b, str) else b
                if isinstance(bs, dict) and bs.get("type") == "record" and isinstance(v, dict):
                    if {f["name"] for f in bs.get("fields", [])} >= set(v):
                        return self.norm(bs, v)
            return v
        if not isinstance(s, dict):
            return v
        connect = s.get("connect.name") or ""
        if connect == "io.debezium.data.VariableScaleDecimal":
            return _variable_decimal(v)
        t = s.get("type")
        if connect.startswith("io.debezium.time."):
            return _time_value(connect, v)
        if t == "record" and isinstance(v, dict):
            return {f["name"]: self.norm(f.get("type"), v.get(f["name"])) for f in s.get("fields", []) if f["name"] in v}
        if t == "array" and isinstance(v, list):
            return [self.norm(s.get("items"), x) for x in v]
        if t == "map" and isinstance(v, dict):
            return {k: self.norm(s.get("values"), x) for k, x in v.items()}
        if isinstance(t, (dict, list)):
            return self.norm(t, v)
        return v


class Decoder:
    def __init__(self, registry_url: str, auth: str = "", timeout: float = 10) -> None:
        self.registry_url = registry_url.rstrip("/")
        self.auth = auth
        self.timeout = timeout
        self._cache: dict[int, tuple[Any, Normalizer, Any]] = {}
        self._lock = threading.Lock()

    def _schema(self, schema_id: int) -> tuple[Any, Normalizer, Any]:
        with self._lock:
            hit = self._cache.get(schema_id)
        if hit:
            return hit
        if not self.registry_url:
            raise RegistryUnavailable("Avro record but no schema registry is configured (VECTORSYNC_SCHEMA_REGISTRY_URL)")
        req = urllib.request.Request(f"{self.registry_url}/schemas/ids/{schema_id}")
        if self.auth:
            req.add_header("Authorization", "Basic " + base64.b64encode(self.auth.encode()).decode())
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as res:
                body = json.loads(res.read().decode("utf-8"))
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                raise DecodeError(f"schema id {schema_id} is not in the registry") from None
            raise RegistryUnavailable(f"schema registry HTTP {exc.code}") from None
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise RegistryUnavailable(f"schema registry: {getattr(exc, 'reason', exc)}") from None
        if body.get("schemaType", "AVRO") != "AVRO":
            raise DecodeError(f"schema id {schema_id} is {body.get('schemaType')}, only Avro is supported")
        raw = json.loads(body["schema"])
        entry = (fastavro.parse_schema(raw), Normalizer(raw), raw)
        with self._lock:
            self._cache[schema_id] = entry
        return entry

    def decode(self, data: bytes | None) -> Any:
        if data is None:
            return None
        if len(data) >= 5 and data[0] == 0:
            parsed, normalizer, raw = self._schema(int.from_bytes(data[1:5], "big"))
            try:
                value = fastavro.schemaless_reader(io.BytesIO(data[5:]), parsed, None)
            except Exception as exc:  # noqa: BLE001 — corrupt body
                raise DecodeError(f"Avro decode failed: {exc}") from exc
            return normalizer.norm(raw, value)
        try:
            return json.loads(data.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            # A plain string key (e.g. a JDBC topic without a key schema) is still a key.
            try:
                return data.decode("utf-8")
            except UnicodeDecodeError:
                raise DecodeError("record is neither Confluent Avro nor JSON") from None

