#!/usr/bin/env python3
"""Private original-memory restore transport for read-only inspection.

Only the explicitly labelled private restoration container is allowed, with no
published ports or operational mounts. Bind only host 127.0.0.1:16380.
This is separate from every synthetic fixture and never invokes SDK startup.
The caller must use read-only GRAPH.RO_QUERY clients. No contents are logged.
"""
import json
import socketserver
import subprocess
import threading

CONTAINER = "graphiti-rs-original-restore-db"
REMOTE = """
import socket, os, threading
s = socket.create_connection(('127.0.0.1', 6379), timeout=5)
s.settimeout(None)
def uplink():
    try:
        while True:
            b = os.read(0, 65536)
            if not b:
                break
            s.sendall(b)
    finally:
        s.shutdown(socket.SHUT_WR)
threading.Thread(target=uplink, daemon=True).start()
while True:
    b = s.recv(65536)
    if not b:
        break
    os.write(1, b)
"""


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        process = subprocess.Popen(
            ["docker", "exec", "-i", CONTAINER, "env", "-i", "PATH=/usr/local/bin:/usr/bin:/bin",
             "python3", "-S", "-u", "-c", REMOTE],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, bufsize=0)

        def up():
            try:
                while True:
                    data = self.request.recv(65536)
                    if not data:
                        break
                    process.stdin.write(data)
            except (OSError, BrokenPipeError):
                pass
            finally:
                process.stdin.close()
        threading.Thread(target=up, daemon=True).start()
        try:
            while True:
                data = process.stdout.read(65536)
                if not data:
                    break
                self.request.sendall(data)
        except OSError:
            pass
        finally:
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
            process.stdout.close()


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    info = json.loads(subprocess.check_output(["docker", "inspect", CONTAINER]))[0]
    labels = info["Config"]["Labels"]
    assert labels["task"] == "graphiti-rs-original-restore"
    assert labels["scope"] == "private-original-memory"
    for name in info["NetworkSettings"]["Networks"]:
        network=json.loads(subprocess.check_output(["docker","network","inspect",name]))[0]
        assert network["Internal"] and network["Labels"]["task"] == "graphiti-rs-original-restore"
    assert info["Config"]["Entrypoint"] == ["redis-server"]
    assert not info["HostConfig"]["PortBindings"]
    with Server(("127.0.0.1", 16380), Handler) as server:
        print("Private restore loopback relay ready; read-only inspection on 16380", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
