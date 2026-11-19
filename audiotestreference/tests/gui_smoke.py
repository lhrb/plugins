#!/usr/bin/env python3
"""Optional real plugin/helper embedding smoke. No DAW.
Adapted from nam-linux-port (GPL-3.0-or-later).
Pass --gain-edit after the CLAP path to also test mouse input with xdotool.
Run input automation on an isolated Xvfb display, not a desktop session.
"""
import ctypes as C
import subprocess
import sys
import time
from clap_smoke import B, CB, D, P, S, U, InputEvents, OutputEvents, instance, main, record

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
ParamInfo = record("ParamInfo", [("id", U), ("flags", U), ("cookie", P),
                                 ("name", C.c_char * 256), ("module", C.c_char * 1024),
                                 ("min_value", D), ("max_value", D), ("default_value", D)])
Params = record("Params", [("count", CB(U, P)), ("get_info", CB(B, P, U, P)),
                            ("get_value", CB(B, P, U, P)), ("value_to_text", CB(B, P, U, D, P, U)),
                            ("text_to_value", P), ("flush", CB(None, P, P, P))])


def check_gain_edit(ptr, plugin, flush_requested, child):
    params_ext = plugin.extension(ptr, b"clap.params")
    assert params_ext, "plugin must expose CLAP parameters"
    params = C.cast(params_ext, C.POINTER(Params)).contents
    ids = {}
    for index in range(params.count(ptr)):
        info = ParamInfo()
        assert params.get_info(ptr, index, C.byref(info))
        ids[info.name] = info.id
    assert {b"Mode", b"Gain"} <= ids.keys(), ids

    inputs = instance(InputEvents, None, lambda _: 0, lambda *_: None)
    outputs = instance(OutputEvents, None, lambda *_: True)

    def value(label):
        raw = D()
        text = C.create_string_buffer(64)
        assert params.get_value(ptr, ids[label], C.byref(raw)), label
        assert params.value_to_text(ptr, ids[label], raw, text, len(text)), label
        return text.value

    def wait_for_value(label, matches, expected):
        deadline = time.monotonic() + 5
        while True:
            plugin.on_main_thread(ptr)
            if flush_requested.is_set():
                flush_requested.clear()
                params.flush(ptr, C.byref(inputs), C.byref(outputs))
            actual = value(label)
            if matches(actual):
                return actual.decode()
            assert time.monotonic() < deadline, f"expected CLAP {label.decode()} {expected}, got {actual!r}"
            time.sleep(0.02)

    def click(x, y):
        subprocess.run(["xdotool", "mousemove", "--window", str(child), str(x), str(y), "click", "1"],
                       check=True, timeout=5)

    assert value(b"Mode") == b"Identity"
    assert float(value(b"Gain")) == 1.0
    click(165, 102)  # Gain mode button at 660×430, scale 1.
    wait_for_value(b"Mode", lambda text: text == b"Gain", "mode Gain after clicking Gain")
    time.sleep(0.25)  # Allow the helper to render the slider after the mode round trip.
    click(185, 186)  # Left quarter of the slider, i.e. approximately -8×.
    gain = wait_for_value(b"Gain", lambda text: -9.0 < float(text) < -7.0,
                          "near -8× after clicking the slider")
    print(f"PASS: real GPUI Gain slider changed the CLAP parameter to {gain}×")


def check_editor(ptr, plugin, flush_requested):
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
        result = tuple(items[index] for index in range(count.value))
        if items:
            x.XFree(items)
        return result

    def wait_for(count):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            plugin.on_main_thread(ptr)
            current = children()
            if len(current) == count:
                return current
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
            (child,) = wait_for(1)
            time.sleep(0.5)  # Allow GPUI to submit initial frames, not just create a window.
            if iteration == 0 and sys.argv[2:] == ["--gain-edit"]:
                check_gain_edit(ptr, plugin, flush_requested, child)
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
    if len(sys.argv) < 2 or sys.argv[2:] not in ([], ["--gain-edit"]):
        raise SystemExit(f"Usage: {sys.argv[0]} CLAP_PATH [--gain-edit]")
    main(check_editor)
