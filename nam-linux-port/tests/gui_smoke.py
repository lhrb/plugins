#!/usr/bin/env python3
"""Optional real X11 CLAP editor lifecycle/automation smoke (needs working Vulkan).

Run on an X11/XWayland display after building the plugin and its helper:
    python3 tests/gui_smoke.py target/debug/libnam_linux_port.so
Opens only a temporary test parent, never changes DAW settings or user assets.
"""
import argparse
import ctypes as C
from pathlib import Path
import time

from clap_smoke import B, CB, D, P, S, U, Smoke, record, require

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


def wait(smoke, predicate, message, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        smoke.pump()
        if predicate():
            return
        time.sleep(0.02)
    raise AssertionError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("library", type=Path)
    args = parser.parse_args()
    x = C.CDLL("libX11.so.6")
    x.XOpenDisplay.argtypes, x.XOpenDisplay.restype = [S], P
    x.XDefaultRootWindow.argtypes, x.XDefaultRootWindow.restype = [P], C.c_ulong
    x.XCreateSimpleWindow.argtypes = [P, C.c_ulong, C.c_int, C.c_int, U, U, U, C.c_ulong, C.c_ulong]
    x.XCreateSimpleWindow.restype = C.c_ulong
    x.XMapWindow.argtypes = [P, C.c_ulong]
    x.XResizeWindow.argtypes = [P, C.c_ulong, U, U]
    x.XGetGeometry.argtypes = [P, C.c_ulong, P, P, P, P, P, P, P]
    x.XDestroyWindow.argtypes = [P, C.c_ulong]
    x.XFlush.argtypes = [P]
    x.XCloseDisplay.argtypes = [P]
    x.XFree.argtypes = [P]
    x.XQueryTree.argtypes = [P, C.c_ulong, P, P, P, P]
    x.XWarpPointer.argtypes = [P, C.c_ulong, C.c_ulong, C.c_int, C.c_int, U, U, C.c_int, C.c_int]
    display = x.XOpenDisplay(None)
    require(display, "cannot connect to DISPLAY; an X11/XWayland session is required")
    parent = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 40, 40, 660, 430, 0, 0, 0)
    x.XMapWindow(display, parent)
    x.XFlush(display)

    def children():
        root, ancestor, count = C.c_ulong(), C.c_ulong(), U()
        items = C.POINTER(C.c_ulong)()
        require(x.XQueryTree(display, parent, C.byref(root), C.byref(ancestor), C.byref(items), C.byref(count)),
                "cannot query test parent's children")
        result = [items[i] for i in range(count.value)]
        if items:
            x.XFree(items)
        return result

    smoke, gui, created = None, None, False
    try:
        smoke = Smoke(args.library.resolve())
        gui = smoke.extension(b"clap.gui", Gui)
        smoke.activate()
        require(gui.supported(smoke.ptr, b"x11", False), "plugin does not support embedded X11")
        for iteration in range(2):
            require(gui.create(smoke.ptr, b"x11", False), "gui.create failed")
            created = True
            scale = 1.0 if iteration == 0 else 1.5
            expected_size = (round(660 * scale), round(430 * scale))
            x.XResizeWindow(display, parent, *expected_size)
            x.XFlush(display)
            require(gui.scale(smoke.ptr, scale), "gui.set_scale failed")
            width, height = U(), U()
            require(gui.size(smoke.ptr, C.byref(width), C.byref(height)), "gui.get_size failed")
            require((width.value, height.value) == expected_size, "unexpected editor dimensions")
            window = Window(b"x11", Handle(x11=parent))
            require(gui.parent(smoke.ptr, C.byref(window)), "gui.set_parent failed")
            # Pinned NIH-plug attaches/shows during set_parent; its embedded
            # show/hide callbacks are historical no-ops returning false.
            # Verify the actual child and interactions instead of treating this
            # framework limitation as a successful CLAP visibility contract.
            gui.show(smoke.ptr)
            wait(smoke, lambda: len(children()) == 1, "GPUI window was not embedded in the host parent")
            root, xpos, ypos = C.c_ulong(), C.c_int(), C.c_int()
            actual_width, actual_height, border, depth = U(), U(), U(), U()
            require(x.XGetGeometry(display, children()[0], C.byref(root), C.byref(xpos), C.byref(ypos),
                                   C.byref(actual_width), C.byref(actual_height), C.byref(border), C.byref(depth)),
                    "cannot query embedded editor geometry")
            require((actual_width.value, actual_height.value) == expected_size,
                    f"GPUI physical size does not match host scale {scale}")
            # Allow its initial snapshot and rendered frame to arrive before input.
            until = time.monotonic() + 0.3
            while time.monotonic() < until:
                smoke.pump()
                time.sleep(0.02)
            if iteration == 0:
                xtest = C.CDLL("libXtst.so.6")
                xtest.XTestFakeButtonEvent.argtypes = [P, U, C.c_int, C.c_ulong]
                previous = smoke.value("input")
                # Scroll over the Input knob in the embedded child.
                x.XWarpPointer(display, 0, parent, 0, 0, 0, 0, 110, 165)
                xtest.XTestFakeButtonEvent(display, 4, 1, 0)
                xtest.XTestFakeButtonEvent(display, 4, 0, 0)
                x.XFlush(display)
                def changed():
                    # A real host applies queued parameter edits during flush/process.
                    smoke.process()
                    return abs(smoke.value("input") - previous) > 1e-5
                wait(smoke, changed, "embedded knob input did not reach the plugin/host")
                print("PASS: embedded GPUI knob → host-thread gesture → plugin parameter")
            gui.destroy(smoke.ptr)
            created = False
            wait(smoke, lambda: not children(), "editor child remained after gui.destroy")
            print(f"PASS: embedded X11 editor open/close cycle {iteration + 1} at {scale}x")
    finally:
        if created:
            gui.destroy(smoke.ptr)
        if smoke is not None:
            smoke.pump()
            smoke.close()
        x.XDestroyWindow(display, parent)
        x.XCloseDisplay(display)


if __name__ == "__main__":
    main()
