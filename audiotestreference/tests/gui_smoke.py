#!/usr/bin/env python3
"""Optional real plugin/helper embedding smoke. No DAW.
Adapted from nam-linux-port (GPL-3.0-or-later).
Pass --gain-edit or --delay-edit after the CLAP path to test mouse input with xdotool.
Run input automation on an isolated Xvfb display, not a desktop session.
"""
import ctypes as C
import subprocess
import sys
import time
from clap_smoke import (B, CB, D, P, S, U, InputEvents, OutputEvents, delay_info,
                        instance, main, parameter_info, record)

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


def check_parameter_edit(ptr, plugin, flush_requested, child, option):
    params, infos = parameter_info(ptr, plugin)
    inputs = instance(InputEvents, None, lambda _: 0, lambda *_: None)
    outputs = instance(OutputEvents, None, lambda *_: True)

    def value(label, *, formatted):
        param_id = infos[label].id
        raw = D()
        assert params.get_value(ptr, param_id, C.byref(raw)), label
        if not formatted:
            return raw.value
        text = C.create_string_buffer(64)
        assert params.value_to_text(ptr, param_id, raw, text, len(text)), label
        return text.value.decode()

    def pump():
        plugin.on_main_thread(ptr)
        if flush_requested.is_set():
            flush_requested.clear()
            params.flush(ptr, C.byref(inputs), C.byref(outputs))

    def wait_for_value(label, matches, expected, *, formatted=True):
        deadline = time.monotonic() + 5
        while True:
            pump()
            actual = value(label, formatted=formatted)
            if matches(actual):
                return actual
            assert time.monotonic() < deadline, f"expected CLAP {label.decode()} {expected}, got {actual!r}"
            time.sleep(0.02)

    def click(x, y):
        subprocess.run(["xdotool", "mousemove", "--window", str(child), str(x), str(y), "click", "1"],
                       check=True, timeout=5)

    def select_mode(x, name):
        click(x, 102)  # Mode row at 660×430, scale 1.
        wait_for_value(b"Mode", lambda text: text == name, f"{name} after clicking {name}")
        time.sleep(0.25)  # Allow the helper to render after the mode round trip.

    assert value(b"Mode", formatted=True) == "Identity"
    if option == "--gain-edit":
        assert float(value(b"Gain", formatted=True)) == 1.0
        select_mode(165, "Gain")
        click(185, 186)  # Left quarter of the track, i.e. approximately -8×.
        gain = wait_for_value(b"Gain", lambda text: -9.0 < float(text) < -7.0,
                              "near -8× after clicking the slider")
        print(f"PASS: real GPUI Gain slider changed the CLAP parameter to {gain}×")
        return

    delay_label = delay_info(infos).name
    assert value(delay_label, formatted=False) == 0
    select_mode(235, "Delay")
    click(185, 186)  # Within the Delay track.
    delay = wait_for_value(delay_label, lambda samples: 0 < samples < 192_000,
                           "a nonzero interior value after clicking the slider", formatted=False)

    select_mode(80, "Identity")
    click(465, 186)  # A different position on the former track must not edit Delay.
    deadline = time.monotonic() + 0.5
    while time.monotonic() < deadline:
        pump()
        assert value(delay_label, formatted=False) == delay, "Delay slider responded in Identity mode"
        time.sleep(0.02)
    print(f"PASS: real GPUI Delay slider changed CLAP to {round(delay)} samples and was inactive in Identity.")


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
            if iteration == 0 and len(sys.argv) == 3:
                check_parameter_edit(ptr, plugin, flush_requested, child, sys.argv[2])
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
    if len(sys.argv) < 2 or sys.argv[2:] not in ([], ["--gain-edit"], ["--delay-edit"]):
        raise SystemExit(f"Usage: {sys.argv[0]} CLAP_PATH [--gain-edit | --delay-edit]")
    main(check_editor)
