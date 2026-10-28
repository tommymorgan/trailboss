"""Exercise inherited lifecycle routing from a live process on an owned PTY."""

import json
import os
from pathlib import Path
import socket
import sys
import time

root = Path(sys.argv[1])
locator = os.environ["HERDR_PANE_ID"]
deadline = time.monotonic() + 30


def write(name, value):
    (root / name).write_text(json.dumps(value) + "\n")


def wait(name):
    while not (root / name).exists():
        if time.monotonic() >= deadline:
            raise TimeoutError(name)
        time.sleep(0.01)


def report(method, state, seq):
    request_id = f"{state}-{method}"
    params = {
        "pane_id": locator,
        "source": "herdr:omp",
        "agent": "omp",
        "seq": seq,
        "agent_session_id": "retained-session",
    }
    if method == "pane.report_agent_session":
        params["session_start_source"] = "startup"
    if method == "pane.report_agent":
        params["state"] = state
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(5)
        connection.connect(os.environ["HERDR_SOCKET_PATH"])
        connection.sendall((json.dumps({"id": request_id, "method": method, "params": params}) + "\n").encode())
        reply = json.loads(connection.makefile("rb").readline())
    assert reply["id"] == request_id, reply
    return reply


write("reporter.ready", {"pid": os.getpid(), "shell_pid": os.getppid(), "locator": locator})
for seq, state in enumerate(("working", "idle"), start=1):
    wait(f"{state}.go")
    replies = []
    if state == "working":
        replies.append(report("pane.report_agent_session", state, 1))
    replies.append(report("pane.report_agent", state, seq + 1))
    write(f"{state}.reply", replies)
wait("reporter.done")
