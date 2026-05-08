from __future__ import annotations

import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

from codex_session_delete.app_paths import resolve_codex_app_dir
from codex_session_delete.api_adapter import ApiAdapter, UnavailableApiAdapter
from codex_session_delete.backup_store import BackupStore
from codex_session_delete.cdp import inject_file
from codex_session_delete.helper_server import HelperServer
from codex_session_delete.models import DeleteResult, DeleteStatus, SessionRef
from codex_session_delete.storage_adapter import SQLiteStorageAdapter


class ApiFirstDeleteService:
    def __init__(self, api_adapter: ApiAdapter, db_path: Path | None, backup_dir: Path):
        self.api_adapter = api_adapter
        self.local_adapter = SQLiteStorageAdapter(db_path, BackupStore(backup_dir)) if db_path else None

    def delete(self, session: SessionRef) -> DeleteResult:
        api_result = self.api_adapter.delete(session)
        if api_result is not None:
            return api_result
        if self.local_adapter is None:
            return DeleteResult(DeleteStatus.FAILED, session.session_id, "No confirmed server API or local database configured")
        return self.local_adapter.delete_local(session)

    def undo(self, token: str) -> DeleteResult:
        if self.local_adapter is None:
            return DeleteResult(DeleteStatus.FAILED, "", "No local backup adapter configured", undo_token=token)
        return self.local_adapter.undo(token)

    def find_archived_thread_by_title(self, title: str) -> SessionRef | None:
        if self.local_adapter is None:
            return None
        return self.local_adapter.find_archived_thread_by_title(title)


class InjectedHelperServer(HelperServer):
    bridge_socket: Any = None


def launch_codex(app_dir: Path, debug_port: int) -> subprocess.Popen | None:
    if app_dir.suffix == ".app":
        subprocess.run(
            ["open", "-a", str(app_dir), "--args", f"--remote-debugging-port={debug_port}", f"--remote-allow-origins=http://127.0.0.1:{debug_port}"],
            check=True,
        )
        return None
    candidates = [app_dir / "Codex.exe", app_dir / "codex.exe"]
    exe = next((path for path in candidates if path.exists()), candidates[-1])
    return subprocess.Popen([
        str(exe),
        f"--remote-debugging-port={debug_port}",
        f"--remote-allow-origins=http://127.0.0.1:{debug_port}",
    ])


def start_helper(service, host: str = "127.0.0.1", port: int = 57321) -> HelperServer:
    server = InjectedHelperServer(host, port, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def inject_with_retry(debug_port: int, script_path: Path, helper_port: int, service: ApiFirstDeleteService, attempts: int = 20, delay: float = 0.5) -> Any:
    last_error: Exception | None = None
    for _ in range(attempts):
        try:
            return inject_file(debug_port, script_path, helper_port, lambda path, payload: handle_bridge_request(service, path, payload))
        except Exception as exc:
            last_error = exc
            time.sleep(delay)
    if last_error is not None:
        raise last_error
    raise RuntimeError("Codex injection failed")


def launch_and_inject(app_dir: Path | None, db_path: Path | None, backup_dir: Path, debug_port: int, helper_port: int) -> tuple[HelperServer, subprocess.Popen | None]:
    resolved_app_dir = resolve_codex_app_dir(app_dir)
    if resolved_app_dir is None:
        raise RuntimeError("Codex App directory not found")
    service = ApiFirstDeleteService(UnavailableApiAdapter(), db_path, backup_dir)
    server = start_helper(service, port=helper_port)
    codex_proc = launch_codex(resolved_app_dir, debug_port)
    script_path = Path(__file__).parent / "inject" / "renderer-inject.js"
    server.bridge_socket = inject_with_retry(debug_port, script_path, server.port, service)
    return server, codex_proc


def handle_bridge_request(service: ApiFirstDeleteService, path: str, payload: dict[str, object]) -> dict[str, object]:
    if path == "/delete":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return service.delete(session).to_dict()
    if path == "/undo":
        return service.undo(str(payload.get("undo_token", ""))).to_dict()
    if path == "/archived-thread":
        session = service.find_archived_thread_by_title(str(payload.get("title", "")))
        return {"session_id": session.session_id, "title": session.title} if session else {"session_id": "", "title": ""}
    return {"status": DeleteStatus.FAILED.value, "session_id": str(payload.get("session_id", "")), "message": "Unknown bridge path"}
