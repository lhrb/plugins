#!/usr/bin/env python3
"""Optional real plugin/helper embedding smoke. No DAW or input automation.
Adapted from nam-linux-port (GPL-3.0-or-later). Run on an isolated Xvfb display.
"""
import ctypes as C
import time
from clap_smoke import B, CB, D, P, S, U, record, main

Gui = record("Gui", [
    ("supported", CB(B, P, S, B)), ("preferred", P), ("create", CB(B, P, S, B)),
    ("destroy", CB(None, P)), ("scale", CB(B, P, D)), ("size", CB(B, P, P, P)),
    ("can_resize", P), ("hints", P), ("adjust", P), ("set_size", P),
    ("parent", CB(B, P, P)), ("transient", P), ("title", P),
    ("show", CB(B, P)), ("hide", CB(B, P)),
])


class Handle(C.Union):
    _fields_ = [("x11", C.c_ulong), ("pointer", P)]


Window = record("Window", [("api", S), ("handle", Handle)])


def check_editor(ptr, plugin):
    x = C.CDLL("libX11.so.6")
    x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [S], P
    x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [P], C.c_ulong
    x.XCreateSimpleWindow.argtypes = [P, C.c_ulong, C.c_int, C.c_int, U, U, U, C.c_ulong, C.c_ulong]
    x.XCreateSimpleWindow.restype = C.c_ulong
    for name in ["XMapWindow", "XDestroyWindow"]:
        getattr(x, name).argtypes = [P, C.c_ulong]
    for name in ["XFlush", "XCloseDisplay", "XFree"]:
        getattr(x, name).argtypes = [P]
    x.XQueryTree.argtypes = [P, C.c_ulong, P, P, P, P]
    display = x.XOpenDisplay(None)
    assert display, "cannot connect to test display"
    parent = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 0, 0, 660, 430, 0, 0, 0)
    x.XMapWindow(display, parent)
    x.XFlush(display)
    gui = C.cast(plugin.extension(ptr, b"clap.gui"), C.POINTER(Gui)).contents
    created = False

    def children():
        root, ancestor, count = C.c_ulong(), C.c_ulong(), U()
        items = C.POINTER(C.c_ulong)()
        assert x.XQueryTree(display, parent, C.byref(root), C.byref(ancestor), C.byref(items), C.byref(count))
        result = count.value
        if items:
            x.XFree(items)
        return result

    def wait_for(count):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            plugin.on_main_thread(ptr)
            if children() == count:
                return
            time.sleep(0.05)
        raise AssertionError(f"expected {count} embedded children, got {children()}")

    try:
        assert gui.supported(ptr, b"x11", False)
        assert not gui.supported(ptr, b"wayland", False)
        for iteration in range(3):
            assert gui.create(ptr, b"x11", False)
            created = True
            assert gui.scale(ptr, 1.0)
            width, height = U(), U()
            assert gui.size(ptr, C.byref(width), C.byref(height))
            assert (width.value, height.value) == (660, 430)
            window = Window(b"x11", Handle(x11=parent))
            assert gui.parent(ptr, C.byref(window))
            # Pinned NIH-plug maps at attachment, NOT via these visibility callbacks.
            print(f"OBSERVATION: CLAP show={gui.show(ptr)}, hide={gui.hide(ptr)} (not certified)")
            wait_for(1)
            time.sleep(0.5)  # Allow GPUI to submit initial frames, not just create a window.
            gui.destroy(ptr)
            created = False
            wait_for(0)
            print(f"PASS: real bundled GPUI X11 child attached/destroyed, cycle {iteration + 1}")
    finally:
        if created:
            gui.destroy(ptr)
        x.XDestroyWindow(display, parent)
        x.XCloseDisplay(display)


if __name__ == "__main__":
    main(check_editor)
