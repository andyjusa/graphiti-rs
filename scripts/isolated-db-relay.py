#!/usr/bin/env python3
"""Synthetic Pi harness transport when internal-network ports are unreachable.

Requires the exact labelled disposable container, without a published port.
Connects only host 127.0.0.1:16379 to that container's 127.0.0.1:6379.
Never loads sitecustomize, Graphiti, provider configuration, or credentials.
"""
import json
import socketserver
import subprocess
import threading

CONTAINER = "graphiti-rs-compat-pi-falkordb-1"
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
    assert labels["task"] == "graphiti-rs-compat-pi-synthetic"
    assert labels["com.docker.compose.project"] == "graphiti-rs-compat-pi"
    assert info["Config"]["Entrypoint"] == ["redis-server"]
    assert not info["HostConfig"]["PortBindings"]
    with Server(("127.0.0.1", 16379), Handler) as server:
        print("Isolated loopback relay ready; latency includes docker-exec pipe", flush=True)
        server.serve_forever()


if __name__ == "__main__":
    main()
