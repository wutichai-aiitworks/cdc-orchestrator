"""Embedding providers for cdc-vectorsync, as LangChain `Embeddings` (what PGVector expects).

Plain urllib so the image needs no provider SDKs:
  ollama  — POST {base_url}/api/embed      (local models such as bge-m3, nomic-embed-text)
  openai  — POST {base_url}/embeddings     (OpenAI or any compatible server: vLLM, LM Studio, …)
  hash    — deterministic local vectors for demos and tests; NOT semantic.

Network/HTTP failures raise EmbeddingUnavailable, which the engine treats as an infrastructure
error (the batch is retried, offsets are not committed).
"""

from __future__ import annotations

import hashlib
import json
import math
import time
import urllib.error
import urllib.request
from typing import Any

from langchain_core.embeddings import Embeddings

PROVIDERS = ("ollama", "openai", "hash")
DEFAULT_MODELS = {"ollama": "bge-m3", "openai": "text-embedding-3-small", "hash": "hash-256"}


class EmbeddingUnavailable(Exception):
    pass


def _post(url: str, body: dict[str, Any], headers: dict[str, str], timeout: float) -> Any:
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST", headers={"Content-Type": "application/json", **headers})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            return json.loads(res.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", "replace")[:300]
        raise EmbeddingUnavailable(f"HTTP {exc.code} from {url}: {detail}") from None
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise EmbeddingUnavailable(f"{url}: {getattr(exc, 'reason', exc)}") from None


def _get(url: str, headers: dict[str, str], timeout: float) -> Any:
    req = urllib.request.Request(url, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            return json.loads(res.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        raise EmbeddingUnavailable(f"HTTP {exc.code} from {url}") from None
    except (urllib.error.URLError, TimeoutError, OSError) as exc:
        raise EmbeddingUnavailable(f"{url}: {getattr(exc, 'reason', exc)}") from None


class ProviderEmbeddings(Embeddings):
    def __init__(self, cfg: dict[str, Any]) -> None:
        self.provider = (cfg.get("provider") or "ollama").strip()
        if self.provider not in PROVIDERS:
            raise ValueError(f"embedding.provider must be one of {', '.join(PROVIDERS)}")
        self.model = (cfg.get("model") or DEFAULT_MODELS[self.provider]).strip()
        self.base_url = (cfg.get("base_url") or "").strip().rstrip("/")
        if self.provider == "ollama" and not self.base_url:
            self.base_url = "http://host.docker.internal:11434"
        if self.provider == "openai" and not self.base_url:
            self.base_url = "https://api.openai.com/v1"
        self.api_key = cfg.get("api_key") or ""
        self.dimensions = int(cfg.get("dimensions") or 0) or None
        self.batch_size = max(1, min(int(cfg.get("batch_size") or 32), 512))
        self.timeout = float(cfg.get("timeout_seconds") or 60)
        self.calls = 0
        self.total_ms = 0.0

    # LangChain interface -------------------------------------------------------------
    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        out: list[list[float]] = []
        for i in range(0, len(texts), self.batch_size):
            out.extend(self._embed(texts[i : i + self.batch_size]))
        return out

    def embed_query(self, text: str) -> list[float]:
        return self._embed([text])[0]

    # ----------------------------------------------------------------------------------
    @property
    def avg_ms(self) -> float | None:
        return round(self.total_ms / self.calls, 1) if self.calls else None

    def _headers(self) -> dict[str, str]:
        return {"Authorization": f"Bearer {self.api_key}"} if self.api_key else {}

    def _embed(self, texts: list[str]) -> list[list[float]]:
        if not texts:
            return []
        t0 = time.monotonic()
        if self.provider == "hash":
            vectors = [_hash_vector(t, self.dimensions or 256) for t in texts]
        elif self.provider == "ollama":
            body: dict[str, Any] = {"model": self.model, "input": texts}
            if self.dimensions:
                body["dimensions"] = self.dimensions
            res = _post(f"{self.base_url}/api/embed", body, self._headers(), self.timeout)
            vectors = res.get("embeddings") if isinstance(res, dict) else None
        else:
            body = {"model": self.model, "input": texts}
            if self.dimensions:
                body["dimensions"] = self.dimensions
            res = _post(f"{self.base_url}/embeddings", body, self._headers(), self.timeout)
            data = res.get("data") if isinstance(res, dict) else None
            vectors = [d["embedding"] for d in sorted(data, key=lambda d: d.get("index", 0))] if isinstance(data, list) else None
        if not isinstance(vectors, list) or len(vectors) != len(texts):
            raise EmbeddingUnavailable(f"{self.provider} returned no embeddings for model {self.model!r}")
        self.calls += 1
        self.total_ms += (time.monotonic() - t0) * 1000
        return vectors

    def list_models(self) -> list[str]:
        if self.provider == "hash":
            return [DEFAULT_MODELS["hash"]]
        if self.provider == "ollama":
            res = _get(f"{self.base_url}/api/tags", self._headers(), 10)
            return sorted(m.get("name", "") for m in res.get("models", []) if m.get("name"))
        res = _get(f"{self.base_url}/models", self._headers(), 10)
        return sorted(m.get("id", "") for m in res.get("data", []) if m.get("id"))

    def probe(self) -> dict[str, Any]:
        t0 = time.monotonic()
        vec = self.embed_query("hyper cdc vector sync probe")
        return {"dimensions": len(vec), "latency_ms": round((time.monotonic() - t0) * 1000, 1)}

    def public(self) -> dict[str, Any]:
        return {"provider": self.provider, "model": self.model, "base_url": self.base_url, "dimensions": self.dimensions}


def _hash_vector(text: str, dims: int) -> list[float]:
    """Bag-of-words feature hashing: stable, cheap, and good enough to show that sync works."""
    vec = [0.0] * dims
    for token in text.lower().split():
        h = int.from_bytes(hashlib.blake2b(token.encode("utf-8"), digest_size=8).digest(), "big")
        vec[h % dims] += 1.0 if (h >> 63) == 0 else -1.0
    norm = math.sqrt(sum(v * v for v in vec)) or 1.0
    return [v / norm for v in vec]
