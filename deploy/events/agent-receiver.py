#!/usr/bin/env python3
"""agent-receiver.py — walgit events 桥的 agent 侧订阅接收器(零依赖,stdlib)。

接收 events 桥 POST 来的事件批次(docs/EVENTS.md 契约),逐批验签、去重、
落盘 JSONL,供 agent 读取/回放。这就是「agent 订阅消费路径」的参考实现:
跑起来即订阅;配合桥的 at-least-once 重试与 `walgit wal ls` 回放,语义完整。

用法:
  WALGIT_EVENTS_SECRET=<桥配置的 webhook_secret> \
  python3 agent-receiver.py [--port 8099] [--path /walgit] \
                            [--out ~/walgit/events.jsonl] [--state ~/walgit/.events-seen]

行为(契约见 docs/EVENTS.md「Consumer checklist」):
  1. 先常量时间校验 X-Walgit-Signature(配置了 secret 时),再解析
  2. 批级去重:X-Walgit-Delivery 已见过 → 200 忽略(桥的重试不产生重复落盘)
  3. 每个事件追加一行 JSONL(含 _walgit.seq / repo / ref_name)
  4. 永远 2xx——落盘失败才 500,让桥重试(at-least-once)
"""

import argparse
import hashlib
import hmac
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

args = None
seen_deliveries: set[str] = set()


def load_seen(path: Path) -> None:
    if path.exists():
        seen_deliveries.update(
            line.strip() for line in path.read_text().splitlines() if line.strip()
        )


def save_seen(path: Path) -> None:
    path.write_text("\n".join(sorted(seen_deliveries)) + "\n")


def verify_signature(secret: str, body: bytes, header: str | None) -> bool:
    if not secret:
        return True
    if not header or not header.startswith("sha256="):
        return False
    expected = hmac.new(secret.encode(), body, hashlib.sha256).hexdigest()
    return hmac.compare_digest(expected, header.removeprefix("sha256="))


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802
        if self.path.split("?")[0] == args.path:
            body = json.dumps({"received_batches": len(seen_deliveries)}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self) -> None:  # noqa: N802
        if self.path.split("?")[0] != args.path:
            self.send_response(404)
            self.end_headers()
            return
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        delivery = self.headers.get("X-Walgit-Delivery", "")
        secret = os.environ.get("WALGIT_EVENTS_SECRET", "")
        if not verify_signature(secret, body, self.headers.get("X-Walgit-Signature")):
            sys.stderr.write("signature mismatch — dropped\n")
            self.send_response(403)
            self.end_headers()
            return
        if delivery and delivery in seen_deliveries:
            sys.stdout.write(f"dup {delivery} — acked, not re-appended\n")
            self.send_response(200)
            self.end_headers()
            return

        try:
            events = json.loads(body)
            if not isinstance(events, list):
                events = [events]
            with open(args.out, "a") as f:
                for ev in events:
                    f.write(json.dumps(ev, ensure_ascii=False) + "\n")
        except (json.JSONDecodeError, OSError) as e:
            sys.stderr.write(f"store failed: {e} — 500 so the bridge retries\n")
            self.send_response(500)
            self.end_headers()
            return

        if delivery:
            seen_deliveries.add(delivery)
            save_seen(Path(args.state))
        sys.stdout.write(f"batch {delivery[:12]}: {len(events)} event(s) appended\n")
        self.send_response(200)
        self.end_headers()

    def log_message(self, *_a) -> None:  # 静默默认访问日志,事件摘要已自行打印
        pass


def main() -> None:
    global args
    home = Path.home() / "walgit"
    ap = argparse.ArgumentParser(description="walgit events receiver")
    ap.add_argument("--port", type=int, default=8099)
    ap.add_argument("--path", default="/walgit")
    ap.add_argument("--out", default=str(home / "events.jsonl"))
    ap.add_argument("--state", default=str(home / ".events-seen"))
    args = ap.parse_args()

    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    load_seen(Path(args.state))
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    print(f"receiver on http://127.0.0.1:{args.port}{args.path} → {args.out}")
    sys.stdout.flush()
    server.serve_forever()


if __name__ == "__main__":
    main()
