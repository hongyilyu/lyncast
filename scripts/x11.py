"""Types keys into a Tier 2 compositor, with only the X11 requests it needs.

    x11.py DISPLAY focus keys SPEC...

DISPLAY is the number test-env.sh's Xvfb wrote to $XDG_RUNTIME_DIR/display. `focus` gives
the X input focus to anvil's window, which anvil needs before it reads keys. Each SPEC taps
one key, named by its X keysym, and `a+b` holds a while it taps b. ctrl, shift, and alt name
the left modifier keys. The keys go through XTEST, and the script returns once the X server
has handled them all.

It speaks the X11 protocol over the display's Unix socket, with no Xlib and no xdotool, and
needs no authorization, as Xvfb under test-env.sh grants none.
"""

import socket
import struct
import sys

KEY_PRESS, KEY_RELEASE = 2, 3
QUERY_TREE, GET_PROPERTY, SET_INPUT_FOCUS, GET_INPUT_FOCUS, QUERY_EXTENSION = 15, 20, 42, 43, 98
FAKE_INPUT = 2
WM_NAME, ANY_PROPERTY_TYPE = 39, 0
REVERT_TO_PARENT, CURRENT_TIME = 2, 0

# anvil reads an X keycode as an evdev code plus 8 and maps it through its own US keymap,
# so these are the evdev codes of a US keyboard plus 8.
KEYCODES = {
    "Escape": 9, "BackSpace": 22, "Tab": 23, "Return": 36, "Control_L": 37,
    "Shift_L": 50, "Alt_L": 64, "space": 65, "Home": 110, "Up": 111, "Left": 113,
    "Right": 114, "End": 115, "Down": 116, "Delete": 119,
    "minus": 20, "equal": 21, "period": 60, "slash": 61, "comma": 59,
    **{d: 9 + i for i, d in enumerate("1234567890", start=1)},
    **{c: 23 + i for i, c in enumerate("qwertyuiop", start=1)},
    **{c: 37 + i for i, c in enumerate("asdfghjkl", start=1)},
    **{c: 51 + i for i, c in enumerate("zxcvbnm", start=1)},
}
ALIASES = {"ctrl": "Control_L", "shift": "Shift_L", "alt": "Alt_L"}


def pad(n):
    return (4 - n % 4) % 4


class X11:
    def __init__(self, display):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(f"/tmp/.X11-unix/X{display}")
        self.sock.sendall(struct.pack("<cxHHHHxx", b"l", 11, 0, 0, 0))
        head = self.recv(8)
        if head[0] != 1:
            sys.exit(f"x11.py: the X server refused the connection: {head!r}")
        body = self.recv(struct.unpack("<H", head[6:8])[0] * 4)
        vendor, = struct.unpack_from("<H", body, 16)
        formats = body[21]
        self.root, = struct.unpack_from("<I", body, 32 + vendor + pad(vendor) + 8 * formats)
        name = b"XTEST"
        reply = self.request(struct.pack("<BxHHxx", QUERY_EXTENSION,
                                         2 + (len(name) + pad(len(name))) // 4, len(name))
                             + name + b"\0" * pad(len(name)))
        if not reply[8]:
            sys.exit("x11.py: the X server has no XTEST")
        self.xtest = reply[9]

    def recv(self, n):
        data = bytearray()
        while len(data) < n:
            chunk = self.sock.recv(n - len(data))
            if not chunk:
                sys.exit("x11.py: the X server closed the connection")
            data += chunk
        return bytes(data)

    def request(self, data):
        self.sock.sendall(data)
        while True:
            head = self.recv(32)
            if head[0] == 1:
                more, = struct.unpack_from("<I", head, 4)
                return head + self.recv(more * 4)
            if head[0] == 0:
                sys.exit(f"x11.py: X error {head[1]} on request {head[10]}")

    def sync(self):
        """GetInputFocus: once its reply comes, the server has handled every request before it."""
        self.request(struct.pack("<BxH", GET_INPUT_FOCUS, 1))

    def children(self, window):
        reply = self.request(struct.pack("<BxHI", QUERY_TREE, 2, window))
        count, = struct.unpack_from("<H", reply, 16)
        return struct.unpack_from(f"<{count}I", reply, 32)

    def name(self, window):
        reply = self.request(struct.pack("<BBHIIIII", GET_PROPERTY, 0, 6, window, WM_NAME,
                                         ANY_PROPERTY_TYPE, 0, 64))
        length, = struct.unpack_from("<I", reply, 16)
        return reply[32:32 + length].decode(errors="replace")

    def focus(self, window):
        self.sock.sendall(struct.pack("<BBHII", SET_INPUT_FOCUS, REVERT_TO_PARENT, 3, window,
                                      CURRENT_TIME))
        self.sync()

    def fake(self, kind, keycode):
        self.sock.sendall(struct.pack("<BBHBBxxIIxxxxxxxxhhxxxxxxxx", self.xtest, FAKE_INPUT, 9,
                                      kind, keycode, 0, 0, 0, 0))


def keycode(name):
    name = ALIASES.get(name, name)
    if name not in KEYCODES:
        sys.exit(f"x11.py: no key {name}")
    return KEYCODES[name]


def main(args):
    if args[1:3] != ["focus", "keys"]:
        sys.exit(__doc__)
    x = X11(args[0])
    anvil = [w for w in x.children(x.root) if x.name(w) == "Smithay"]
    if len(anvil) != 1:
        sys.exit(f"x11.py: {len(anvil)} windows named Smithay under the root")
    x.focus(anvil[0])
    for spec in args[3:]:
        *held, tapped = [keycode(name) for name in spec.split("+")]
        for code in held:
            x.fake(KEY_PRESS, code)
        x.fake(KEY_PRESS, tapped)
        x.fake(KEY_RELEASE, tapped)
        for code in reversed(held):
            x.fake(KEY_RELEASE, code)
    x.sync()


if __name__ == "__main__":
    main(sys.argv[1:])
