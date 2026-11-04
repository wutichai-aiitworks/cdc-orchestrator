"""HTTP contract shared by the Copy & Sync sidecars (same as before, plus history and tests):

  GET  /health                        {"status": "ok"}
  GET  /v1/discover | /v1/capability  tool description (+ versions)
  GET  /v1/jobs[?profile_id=]         runs, newest first
  POST /v1/jobs                       start a run → 202 (409 when the transfer is already running)
  GET  /v1/jobs/{id}                  one run
  GET  /v1/jobs/{id}/logs             {"id", "log"}
  POST /v1/jobs/{id}/stop             stop a run
  DELETE /v1/jobs/{id}                stop a run (kept for older orchestrators)
  POST /v1/test-connection            {connection, side?, options?} → {ok, target, facts[], warning?, error?, hint?}
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Callable
from urllib.parse import parse_qs, urlparse

from jobstore import BusyError, JobStore, NotFound
from probes import BadRequest


@dataclass
class Tool:
    tool_id: str
    service: str
    store: JobStore
    capability: Callable[[], dict[str, Any]]
    start: Callable[[dict[str, Any]], dict[str, Any]]
    test: Callable[[dict[str, Any]], dict[str, Any]]
    api_token: str = ""
    validate: Callable[[dict[str, Any]], None] = field(default=lambda body: None)


def make_handler(tool: Tool) -> type[BaseHTTPRequestHandler]:
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt: str, *args: Any) -> None:
            return

        def _auth_ok(self) -> bool:
            if not tool.api_token:
                return True
            auth = self.headers.get("Authorization", "")
            if auth.startswith("Bearer ") and auth[7:].strip() == tool.api_token:
                return True
            return self.headers.get("X-API-Token", "").strip() == tool.api_token

        def _json(self, code: int, payload: Any) -> None:
            body = json.dumps(payload).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _body(self) -> dict[str, Any]:
            length = int(self.headers.get("Content-Length", "0") or "0")
            raw = self.rfile.read(length) if length else b"{}"
            try:
                data = json.loads(raw.decode("utf-8") or "{}")
            except (UnicodeDecodeError, json.JSONDecodeError):
                raise BadRequest("request body must be JSON") from None
            if not isinstance(data, dict):
                raise BadRequest("request body must be a JSON object")
            return data

        def _route(self, method: str) -> None:
            if not self._auth_ok():
                self._json(401, {"error": "unauthorized"})
                return
            url = urlparse(self.path)
            parts = [p for p in url.path.split("/") if p]
            query = parse_qs(url.query)
            try:
                self._dispatch(method, parts, query)
            except BadRequest as exc:
                self._json(400, {"error": str(exc)})
            except NotFound as exc:
                self._json(404, {"error": str(exc)})
            except BusyError as exc:
                self._json(409, {"error": str(exc)})
            except Exception as exc:  # noqa: BLE001
                self._json(500, {"error": f"{type(exc).__name__}: {exc}"})

        def _dispatch(self, method: str, parts: list[str], query: dict[str, list[str]]) -> None:
            store = tool.store
            if method == "GET" and parts == ["health"]:
                self._json(200, {"status": "ok"})
            elif method == "GET" and parts in (["v1", "capability"], ["v1", "discover"]):
                out = {"service": tool.service, **tool.capability()}
                if parts[1] == "discover":
                    out.update({"contract_version": "1", "tool_id": tool.tool_id, "sidecar_ready": True, "orchestrator_contract": f"/cdc/{tool.tool_id}/contract"})
                self._json(200, out)
            elif parts == ["v1", "jobs"] and method == "GET":
                profile_id = (query.get("profile_id") or [None])[0]
                self._json(200, {"jobs": store.list(profile_id)})
            elif parts == ["v1", "jobs"] and method == "POST":
                body = self._body()
                if not body.get("source") or not body.get("target"):
                    raise BadRequest("source and target are required")
                tool.validate(body)
                self._json(202, tool.start(body))
            elif parts == ["v1", "test-connection"] and method == "POST":
                self._json(200, tool.test(self._body()))
            elif len(parts) == 3 and parts[:2] == ["v1", "jobs"]:
                if method == "GET":
                    self._json(200, store.meta(parts[2]))
                elif method == "DELETE":
                    self._json(200, store.stop(parts[2]))
                else:
                    self._json(405, {"error": "method not allowed"})
            elif len(parts) == 4 and parts[:2] == ["v1", "jobs"] and parts[3] == "logs" and method == "GET":
                self._json(200, {"id": parts[2], "log": store.tail_log(parts[2])})
            elif len(parts) == 4 and parts[:2] == ["v1", "jobs"] and parts[3] == "stop" and method == "POST":
                self._json(200, store.stop(parts[2]))
            else:
                self._json(404, {"error": "not found"})

        def do_GET(self) -> None:  # noqa: N802
            self._route("GET")

        def do_POST(self) -> None:  # noqa: N802
            self._route("POST")

        def do_DELETE(self) -> None:  # noqa: N802
            self._route("DELETE")

    return Handler


def serve(tool: Tool, host: str, port: int) -> None:
    tool.store.jobs_dir.mkdir(parents=True, exist_ok=True)
    server = ThreadingHTTPServer((host, port), make_handler(tool))
    print(f"{tool.service} listening on {host}:{port} (pid {os.getpid()})", flush=True)
    server.serve_forever()
