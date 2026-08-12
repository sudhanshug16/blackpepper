#!/usr/bin/env python3
"""Small loopback HTTP listener whose cwd can be attributed to a workspace."""

from __future__ import annotations

import http.server
import pathlib
import sys


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - required by BaseHTTPRequestHandler
        body = b"blackpepper-tui-e2e\n"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args: object) -> None:
        return


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: listener.py PORT_FILE")
    port_file = pathlib.Path(sys.argv[1])
    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler) as server:
        port_file.write_text(f"{server.server_port}\n", encoding="ascii")
        server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
