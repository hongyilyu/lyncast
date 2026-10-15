#!/usr/bin/env python3
"""Prints the globals of a running Wayland compositor, one "interface version" per line.

Connects to $WAYLAND_DISPLAY (a name in $XDG_RUNTIME_DIR, or an absolute path) and
does one wl_display.sync roundtrip, so success also shows the compositor is serving."""
import os
import socket
import struct
import sys


def recv_message(sock, buf):
    while len(buf) < 8 or len(buf) < struct.unpack_from("=I", buf, 4)[0] >> 16:
        chunk = sock.recv(4096)
        if not chunk:
            sys.exit("wl-globals: the compositor closed the connection")
        buf += chunk
    obj, word = struct.unpack_from("=II", buf)
    size = word >> 16
    return obj, word & 0xFFFF, buf[8:size], buf[size:]


def string_at(body, offset):
    """Returns (text, offset after it) for a Wayland string argument."""
    n = struct.unpack_from("=I", body, offset)[0]
    return body[offset + 4 : offset + 4 + n - 1].decode(), offset + 4 + (n + 3) // 4 * 4


def main():
    name = os.environ["WAYLAND_DISPLAY"]
    path = name if name.startswith("/") else os.path.join(os.environ["XDG_RUNTIME_DIR"], name)
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(1)
    try:
        sock.connect(path)
        # wl_display (object 1): get_registry(new_id 2), then sync(new_id 3).
        sock.sendall(struct.pack("=6I", 1, 12 << 16 | 1, 2, 1, 12 << 16 | 0, 3))
        buf = b""
        while True:
            obj, opcode, body, buf = recv_message(sock, buf)
            if obj == 2 and opcode == 0:  # wl_registry.global(name, interface, version)
                interface, end = string_at(body, 4)
                print(interface, struct.unpack_from("=I", body, end)[0])
            elif obj == 3:  # wl_callback.done: every global has been sent
                return
            elif obj == 1 and opcode == 0:  # wl_display.error(object, code, message)
                sys.exit(f"wl-globals: protocol error: {string_at(body, 8)[0]}")
    except OSError as e:
        sys.exit(f"wl-globals: {path}: {e}")


if __name__ == "__main__":
    main()
