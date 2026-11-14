"""Unit tests for cdc-vectorsync (no Kafka, no Postgres). Needs requirements.txt installed:
  cd docker/vectorsync && python3 -m unittest -q
Skipped automatically when pydebeziumai is not importable (e.g. the plain `npm run check` host).
"""

from __future__ import annotations

import base64
import io
import json
import unittest
from decimal import Decimal

try:
    import bridge
    import engine
    from decode import Decoder, Normalizer
    from embeddings import EmbeddingUnavailable, ProviderEmbeddings

    HAVE_DEPS = True
except ImportError:  # pragma: no cover
    HAVE_DEPS = False


def envelope(op: str, before=None, after=None):
    return {"op": op, "before": before, "after": after, "ts_ms": 1763107200000, "source": {"table": "orders"}}


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class BridgeTests(unittest.TestCase):
    def spec(self, **kw):
        base = {"name": "t", "projection": {"default": {"exclude_fields": ["secret"]}}, "id_strategy": {}}
        base.update(kw)
        return base

    def test_envelope_insert_builds_document_with_stable_id(self):
        b = bridge.make_builder(self.spec())
        ev = bridge.to_event("demo.bookings.orders", 0, {"id": 42}, envelope("c", after={"id": 42, "note": "hello", "secret": "x"}))
        self.assertEqual(bridge.doc_id(b, ev), "bookings.orders:42")
        doc = bridge.build(b, ev, soft_delete=False)
        self.assertIn("note: hello", doc.page_content)
        self.assertNotIn("secret", doc.page_content)
        self.assertNotIn("secret", doc.metadata)
        self.assertEqual(doc.metadata["_table"], "orders")
        self.assertEqual(doc.metadata["_sync"], "t")

    def test_delete_uses_before_row_id(self):
        b = bridge.make_builder(self.spec())
        ev = bridge.to_event("demo.bookings.orders", 0, {"id": 7}, envelope("d", before={"id": 7}))
        self.assertEqual(bridge.doc_id(b, ev), "bookings.orders:7")

    def test_flat_jdbc_row_is_a_snapshot_read(self):
        ev = bridge.to_event("inc.orders", 0, None, {"id": 1, "name": "a"})
        self.assertEqual(ev.payload.op, "r")
        b = bridge.make_builder(self.spec(id_strategy={"pk_fields": ["id"]}))
        self.assertEqual(bridge.doc_id(b, ev), "inc.orders:1")

    def test_tombstone_truncate_and_ddl_are_skipped(self):
        self.assertIsNone(bridge.to_event("t.s.x", 0, {"id": 1}, None))
        self.assertIsNone(bridge.to_event("t.s.x", 0, None, {"op": "t", "before": None, "after": None}))
        self.assertIsNone(bridge.to_event("t", 0, None, {"ddl": "CREATE TABLE x()", "tableChanges": []}))

    def test_missing_key_without_pk_fields_is_a_data_error(self):
        b = bridge.make_builder(self.spec())
        ev = bridge.to_event("inc.orders", 0, None, {"name": "a"})
        with self.assertRaises(bridge.DataError):
            bridge.doc_id(b, ev)

    def test_template_and_per_table_override(self):
        b = bridge.make_builder(self.spec(projection={"default": {}, "tables": {"orders": {"content_template": "Order $id: $note", "metadata_fields": ["id"]}}}))
        ev = bridge.to_event("demo.bookings.orders", 0, {"id": 1}, envelope("c", after={"id": 1, "note": "rush", "x": 2}))
        doc = bridge.build(b, ev, soft_delete=False)
        self.assertEqual(doc.page_content, "Order 1: rush")
        self.assertEqual(doc.metadata["id"], 1)
        self.assertNotIn("x", doc.metadata)

    def test_template_with_unknown_column_is_dead_lettered_not_embedded(self):
        b = bridge.make_builder(self.spec(projection={"default": {"content_template": "$nope"}}))
        ev = bridge.to_event("demo.bookings.orders", 0, {"id": 1}, envelope("c", after={"id": 1}))
        with self.assertRaises(bridge.DataError):
            bridge.build(b, ev, soft_delete=False)

    def test_template_checks(self):
        self.assertIsNone(bridge.check_template("$a and ${b} cost $$5"))
        self.assertIsNotNone(bridge.check_template("bad $"))
        self.assertEqual(bridge.template_fields("$a ${b} $$c"), ["a", "b"])

    def test_soft_delete_builds_marked_document(self):
        b = bridge.make_builder(self.spec())
        ev = bridge.to_event("demo.bookings.orders", 0, {"id": 3}, envelope("d", before={"id": 3, "note": "gone"}))
        doc = bridge.build(b, ev, soft_delete=True)
        self.assertTrue(doc.metadata["_is_deleted"])


class FakeAdapter:
    """VectorStore stand-in: the first upsert_batch may fail; per-row retries honour `reject`."""

    def __init__(self, fail_batch=None, reject=()):
        self.upserted, self.deleted, self.parents_deleted = [], [], []
        self.fail_batch = fail_batch
        self.reject = set(reject)
        self.calls = 0

    def upsert_batch(self, docs):
        self.calls += 1
        if self.fail_batch and self.calls == 1:
            raise self.fail_batch
        for d in docs:
            if d.id in self.reject or d.metadata.get("_parent_id") in self.reject:
                raise EmbeddingUnavailable("HTTP 400 from http://ollama/api/embed: input too long")
        self.upserted.extend(d.id for d in docs)

    def upsert(self, doc):
        self.upsert_batch([doc])

    def delete_batch(self, ids):
        self.deleted.extend(ids)

    def delete_parents(self, ids):
        self.parents_deleted.extend(ids)


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class ApplyBatchTests(unittest.TestCase):
    def setUp(self):
        self.builder = bridge.make_builder({"name": "t", "projection": {}, "id_strategy": {}})

    def change(self, op, pk, offset, **row):
        row = {"id": pk, **row}
        ev = bridge.to_event("demo.s.t", 0, {"id": pk}, envelope(op, before=row if op == "d" else None, after=None if op == "d" else row))
        return bridge.Change(ev, bridge.doc_id(self.builder, ev), op, "demo.s.t", 0, offset)

    def test_compaction_last_change_wins(self):
        a = FakeAdapter()
        changes = [self.change("c", 1, 0, v="a"), self.change("u", 1, 1, v="b"), self.change("d", 1, 2), self.change("c", 2, 3)]
        stats = engine.apply_batch(changes, self.builder, a, False, lambda c, m: self.fail(m))
        self.assertEqual(a.deleted, ["s.t:1"])
        self.assertEqual(a.upserted, ["s.t:2"])
        self.assertEqual(stats, {"upserted": 1, "deleted": 1, "chunks": 1})

    def test_store_outage_is_infra_error_and_nothing_dead_lettered(self):
        dead = []
        a = FakeAdapter(fail_batch=ConnectionError("pg down"))
        with self.assertRaises(engine.InfraError):
            engine.apply_batch([self.change("c", 1, 0)], self.builder, a, False, lambda c, m: dead.append(m))
        self.assertEqual(dead, [])

    def test_embedding_outage_is_infra(self):
        a = FakeAdapter(fail_batch=EmbeddingUnavailable("http://ollama: Connection refused"))
        with self.assertRaises(engine.InfraError):
            engine.apply_batch([self.change("c", 1, 0)], self.builder, a, False, lambda c, m: None)

    def test_client_error_isolates_the_bad_row(self):
        dead = []
        a = FakeAdapter(fail_batch=EmbeddingUnavailable("HTTP 400 from x: too long"), reject={"s.t:2"})
        stats = engine.apply_batch([self.change("c", 1, 0), self.change("c", 2, 1)], self.builder, a, False, lambda c, m: dead.append(c.doc_id))
        self.assertEqual(a.upserted, ["s.t:1"])
        self.assertEqual(dead, ["s.t:2"])
        self.assertEqual(stats["upserted"], 1)

    def test_nul_bytes_are_stripped(self):
        a = FakeAdapter()
        captured = []
        a.upsert_batch = lambda docs: captured.extend(docs)
        engine.apply_batch([self.change("c", 1, 0, v="a\x00b")], self.builder, a, False, lambda c, m: None)
        self.assertNotIn("\x00", captured[0].page_content)


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class SpecTests(unittest.TestCase):
    def good(self):
        return {"id": "x", "topics": ["demo.s.t"], "embedding": {"provider": "hash"}, "target": {"host": "h", "database": "d", "username": "u", "collection": "orders_docs"}}

    def test_valid(self):
        engine.validate_spec(self.good())

    def test_rejects(self):
        for mutate, msg in [
            (lambda s: s.update(topics=[]), "topics"),
            (lambda s: s.update(topics=["bad topic"]), "topic name"),
            (lambda s: s["target"].update(collection="-x"), "collection"),
            (lambda s: s["embedding"].update(provider="nope"), "provider"),
            (lambda s: s.update(projection={"default": {"content_template": "oops $"}}), "template"),
            (lambda s: s.update(delete_mode="maybe"), "delete_mode"),
        ]:
            s = self.good()
            mutate(s)
            with self.assertRaisesRegex(engine.BadSpec, msg):
                engine.validate_spec(s)

    def test_pg_url_escapes_credentials(self):
        url = engine.pg_url({"host": "h", "port": "5433", "database": "d b", "username": "u@x", "password": "p:/@", "sslmode": "require"})
        self.assertEqual(url, "postgresql+psycopg://u%40x:p%3A%2F%40@h:5433/d%20b?sslmode=require")


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class DecodeTests(unittest.TestCase):
    SCHEMA = {
        "type": "record", "name": "Envelope", "namespace": "demo.s.t",
        "fields": [
            {"name": "before", "type": ["null", {"type": "record", "name": "Value", "fields": [
                {"name": "id", "type": "int"},
                {"name": "created", "type": ["null", {"type": "long", "connect.name": "io.debezium.time.MicroTimestamp"}]},
                {"name": "day", "type": ["null", {"type": "int", "connect.name": "io.debezium.time.Date"}]},
                {"name": "amount", "type": ["null", {"type": "record", "name": "VariableScaleDecimal", "namespace": "io.debezium.data", "connect.name": "io.debezium.data.VariableScaleDecimal", "fields": [{"name": "scale", "type": "int"}, {"name": "value", "type": "bytes"}]}]},
            ]}]},
            {"name": "after", "type": ["null", "Value"]},
            {"name": "op", "type": "string"},
        ],
    }

    def test_avro_roundtrip_with_debezium_types(self):
        import fastavro

        rec = {"before": None, "after": {"id": 1, "created": 1763107200000000, "day": 20406, "amount": {"scale": 2, "value": (12345).to_bytes(2, "big", signed=True)}}, "op": "c"}
        buf = io.BytesIO()
        fastavro.schemaless_writer(buf, fastavro.parse_schema(self.SCHEMA), rec)
        raw = b"\x00" + (7).to_bytes(4, "big") + buf.getvalue()
        d = Decoder("http://registry.invalid")
        d._cache[7] = (fastavro.parse_schema(self.SCHEMA), Normalizer(self.SCHEMA), self.SCHEMA)
        out = d.decode(raw)
        self.assertEqual(out["after"]["created"], "2025-11-14T08:00:00")
        self.assertEqual(out["after"]["day"], "2025-11-14")
        self.assertEqual(out["after"]["amount"], Decimal("123.45"))
        self.assertEqual(out["op"], "c")

    def test_json_and_plain_string(self):
        d = Decoder("")
        self.assertEqual(d.decode(json.dumps({"id": 1}).encode()), {"id": 1})
        self.assertEqual(d.decode(b"abc"), "abc")
        self.assertIsNone(d.decode(None))

    def test_variable_decimal_base64(self):
        n = Normalizer(self.SCHEMA)
        v = n.norm({"type": "record", "name": "X", "connect.name": "io.debezium.data.VariableScaleDecimal", "fields": []}, {"scale": 1, "value": base64.b64encode(b"\x0f").decode()})
        self.assertEqual(v, Decimal("1.5"))


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class EmbeddingTests(unittest.TestCase):
    def test_hash_provider_is_deterministic_and_normalised(self):
        e = ProviderEmbeddings({"provider": "hash", "dimensions": 64})
        a, b = e.embed_query("hello world"), e.embed_query("hello world")
        self.assertEqual(a, b)
        self.assertEqual(len(a), 64)
        self.assertAlmostEqual(sum(x * x for x in a), 1.0, places=6)

    def test_unreachable_ollama_raises_unavailable(self):
        e = ProviderEmbeddings({"provider": "ollama", "base_url": "http://127.0.0.1:9", "timeout_seconds": 2})
        with self.assertRaises(EmbeddingUnavailable):
            e.embed_query("x")



@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class ChunkingTests(unittest.TestCase):
    def test_split_respects_size_and_overlaps(self):
        import chunking

        text = " ".join(f"Sentence number {i} about warranty and tyres." for i in range(200))
        parts = chunking.split_text(text, max_chars=500, overlap=60)
        self.assertGreater(len(parts), 5)
        self.assertTrue(all(len(p) <= 500 + 60 for p in parts))
        self.assertEqual(chunking.split_text("short", 500, 60), ["short"])
        self.assertEqual(chunking.split_text("", 500, 60), [])
        # overlap: each later chunk starts with words from the end of the previous one
        self.assertIn(parts[1].split()[0], parts[0])

    def test_merge_small_packs_paragraphs(self):
        import chunking

        self.assertEqual(chunking.merge_small(["a" * 40, "b" * 40, "c" * 40], 100), ["a" * 40 + "\n\n" + "b" * 40, "c" * 40])


@unittest.skipUnless(HAVE_DEPS, "vectorsync requirements not installed")
class ShaperTests(unittest.TestCase):
    def change(self, spec, row, op="c"):
        b = bridge.make_builder(spec)
        ev = bridge.to_event("demo.s.manuals", 0, {"id": row["id"]}, envelope(op, after=row if op != "d" else None, before=row if op == "d" else None))
        return b, bridge.Change(ev, bridge.doc_id(b, ev), op, "demo.s.manuals", 0, 1)

    def test_text_chunking_gives_parented_chunk_ids_and_replaces_old_chunks(self):
        spec = {"name": "t", "projection": {"tables": {"manuals": {"chunking": {"enabled": True, "max_chars": 300, "overlap": 30}}}}}
        b, c = self.change(spec, {"id": 7, "body": "Tyres need rotation. " * 60})
        a = FakeAdapter()
        stats = engine.apply_batch([c], b, a, False, lambda c, m: self.fail(m), engine.Shaper(spec))
        self.assertGreater(stats["chunks"], 1)
        self.assertEqual(stats["upserted"], 1)
        self.assertEqual(a.upserted[0], "s.manuals:7#c0")
        self.assertEqual(a.parents_deleted, ["s.manuals:7"], "old chunks of the row are removed first")

    def test_short_text_stays_one_vector(self):
        spec = {"name": "t", "projection": {"tables": {"manuals": {"chunking": {"enabled": True}}}}}
        b, c = self.change(spec, {"id": 1, "body": "short"})
        a = FakeAdapter()
        engine.apply_batch([c], b, a, False, lambda c, m: None, engine.Shaper(spec))
        self.assertEqual(a.upserted, ["s.manuals:1"])

    def test_delete_removes_all_chunks_of_the_row(self):
        spec = {"name": "t", "projection": {"tables": {"manuals": {"chunking": {"enabled": True}}}}}
        b, c = self.change(spec, {"id": 3, "body": "x"}, op="d")
        a = FakeAdapter()
        engine.apply_batch([c], b, a, False, lambda c, m: None, engine.Shaper(spec))
        self.assertEqual(a.parents_deleted, ["s.manuals:3"])
        self.assertEqual(a.deleted, ["s.manuals:3"])

    def test_document_column_must_hold_bytes(self):
        import docling_tx

        with self.assertRaises(docling_tx.DocumentError):
            docling_tx.to_bytes("not base64 !!")
        self.assertEqual(docling_tx.to_bytes("\\x2550"), b"%P")
        self.assertEqual(docling_tx.sniff(b"%PDF-1.4 ...", None), "document.pdf")
        with self.assertRaises(docling_tx.DocumentError):
            docling_tx.sniff(b"\x00\x01garbage", None)

    def test_document_column_needs_docling_in_the_image(self):
        import docling_tx

        spec = {"id": "x", "topics": ["demo.s.t"], "embedding": {"provider": "hash"}, "target": {"host": "h", "database": "d", "username": "u", "collection": "c"},
                "projection": {"tables": {"t": {"document": {"column": "file"}}}}}
        saved = docling_tx.AVAILABLE
        try:
            docling_tx.AVAILABLE = False
            with self.assertRaisesRegex(engine.BadSpec, "VECTORSYNC_DOCLING"):
                engine.validate_spec(spec)
        finally:
            docling_tx.AVAILABLE = saved

    def test_chunk_size_limits(self):
        spec = {"id": "x", "topics": ["demo.s.t"], "embedding": {"provider": "hash"}, "target": {"host": "h", "database": "d", "username": "u", "collection": "c"},
                "projection": {"tables": {"t": {"chunking": {"enabled": True, "max_chars": 50}}}}}
        with self.assertRaisesRegex(engine.BadSpec, "200-20000"):
            engine.validate_spec(spec)


try:
    import docling_tx as _dt

    HAVE_DOCLING = HAVE_DEPS and _dt.AVAILABLE
except ImportError:  # pragma: no cover
    HAVE_DOCLING = False


@unittest.skipUnless(HAVE_DOCLING, "Docling not installed (cdc-vectorsync:docling image only)")
class DoclingTests(unittest.TestCase):
    def docx_bytes(self):
        import io
        import docx

        d = docx.Document()
        d.add_heading("Warranty policy", 1)
        d.add_paragraph("Batteries are covered for 24 months or 40,000 km.")
        d.add_heading("Exclusions", 2)
        d.add_paragraph("Flood damage is not covered.")
        buf = io.BytesIO()
        d.save(buf)
        return buf.getvalue()

    def test_docx_in_a_bytes_column_becomes_heading_aware_chunks(self):
        spec = {"name": "t", "projection": {"tables": {"manuals": {"document": {"column": "file", "filename_column": "fname"}, "chunking": {"max_chars": 300, "overlap": 0}}}}}
        b = bridge.make_builder(spec)
        row = {"id": 9, "fname": "policy.docx", "file": self.docx_bytes()}
        ev = bridge.to_event("demo.s.manuals", 0, {"id": 9}, envelope("c", after=row))
        c = bridge.Change(ev, bridge.doc_id(b, ev), "c", "demo.s.manuals", 0, 1)
        a = FakeAdapter()
        captured = []
        a.upsert_batch = lambda docs: captured.extend(docs)
        engine.apply_batch([c], b, a, False, lambda c, m: self.fail(m), engine.Shaper(spec))
        self.assertGreaterEqual(len(captured), 1)
        text = "\n".join(d.page_content for d in captured)
        self.assertIn("Batteries are covered", text)
        self.assertIn("Exclusions", text)
        meta = captured[0].metadata
        self.assertEqual(meta["_content"], "document")
        self.assertEqual(meta["_doc_format"], "docx")
        self.assertEqual(meta["_parent_id"], "s.manuals:9")
        self.assertNotIn("file", meta, "file bytes never become metadata")
        self.assertEqual(meta["fname"], "policy.docx")

    def test_corrupt_file_is_dead_lettered(self):
        spec = {"name": "t", "projection": {"tables": {"manuals": {"document": {"column": "file"}}}}}
        b = bridge.make_builder(spec)
        row = {"id": 1, "file": b"PK\x03\x04 not really a zip"}
        ev = bridge.to_event("demo.s.manuals", 0, {"id": 1}, envelope("c", after=row))
        c = bridge.Change(ev, bridge.doc_id(b, ev), "c", "demo.s.manuals", 0, 1)
        dead = []
        engine.apply_batch([c], b, FakeAdapter(), False, lambda c, m: dead.append(m), engine.Shaper(spec))
        self.assertEqual(len(dead), 1)

if __name__ == "__main__":
    unittest.main()
