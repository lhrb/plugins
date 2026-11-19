#!/usr/bin/env python3
"""Standalone CLAP ABI shell smoke, adapted from nam-linux-port (GPL-3.0-or-later).
No DAW, editor, user assets, or analyzer. Not Identity certification.
"""
import ctypes as C
from pathlib import Path
import sys
from threading import Event

P = C.c_void_p
U = C.c_uint32
B = C.c_bool
D = C.c_double
F = C.c_float
S = C.c_char_p
CB = C.CFUNCTYPE


def record(name, fields):
    return type(name, (C.Structure,), {"_fields_": fields})


Version = record("Version", [("major", U), ("minor", U), ("revision", U)])
Entry = record("Entry", [("version", Version), ("init", CB(B, S)),
                         ("deinit", CB(None)), ("get_factory", CB(P, S))])
Descriptor = record("Descriptor", [("version", Version)] +
                    [(key, S) for key in ("id", "name", "vendor", "url", "manual_url",
                                         "support_url", "version_string", "description")] +
                    [("features", C.POINTER(S))])
Factory = record("Factory", [("count", CB(U, P)), ("descriptor", CB(P, P, U)),
                             ("create", CB(P, P, P, S))])
Plugin = record("Plugin", [("descriptor", P), ("data", P),
                           ("init", CB(B, P)), ("destroy", CB(None, P)),
                           ("activate", CB(B, P, D, U, U)), ("deactivate", CB(None, P)),
                           ("start", CB(B, P)), ("stop", CB(None, P)),
                           ("reset", CB(None, P)), ("process", CB(C.c_int32, P, P)),
                           ("extension", CB(P, P, S)), ("on_main_thread", CB(None, P))])
Host = record("Host", [("version", Version), ("data", P)] +
              [(key, S) for key in ("name", "vendor", "url", "version_string")] +
              [("extension", CB(P, P, S)), ("restart", CB(None, P)),
               ("process", CB(None, P)), ("callback", CB(None, P))])
ThreadCheck = record("ThreadCheck", [("main", CB(B, P)), ("audio", CB(B, P))])
HostParams = record("HostParams", [("rescan", CB(None, P, U)), ("clear", CB(None, P, U, U)),
                                   ("request_flush", CB(None, P))])
InputEvents = record("InputEvents", [("ctx", P), ("size", CB(U, P)),
                                     ("get", CB(P, P, U))])
OutputEvents = record("OutputEvents", [("ctx", P), ("push", CB(B, P, P))])
Audio = record("Audio", [("data32", C.POINTER(C.POINTER(F))), ("data64", P),
                         ("channels", U), ("latency", U), ("constant_mask", C.c_uint64)])
Process = record("Process", [("steady_time", C.c_int64), ("frames", U), ("transport", P),
                             ("inputs", C.POINTER(Audio)), ("outputs", C.POINTER(Audio)),
                             ("input_count", U), ("output_count", U),
                             ("in_events", C.POINTER(InputEvents)),
                             ("out_events", C.POINTER(OutputEvents))])
PortInfo = record("PortInfo", [("id", U), ("name", C.c_char * 256), ("flags", U),
                               ("channels", U), ("type", S), ("pair", U)])
Ports = record("Ports", [("count", CB(U, P, B)), ("get", CB(B, P, U, B, P))])
def instance(cls, *values):
    """Wrap callbacks and keep them alive with their owning C structure."""
    return cls(*(typ(value) if isinstance(typ, type) and
                 issubclass(typ, C._CFuncPtr) else value
                 for (_, typ), value in zip(cls._fields_, values)))


def main(editor_check=None):
    path = Path(sys.argv[1]).resolve()
    audio_thread = False
    check = instance(ThreadCheck, lambda _: not audio_thread, lambda _: audio_thread)
    flush_requested = Event()
    params = instance(HostParams, lambda *_: None, lambda *_: None, lambda _: flush_requested.set())
    extensions = {b"clap.thread-check": C.addressof(check), b"clap.params": C.addressof(params)}
    host = instance(Host, Version(1, 2, 2), None, b"Shell smoke", b"AudioTestReference", b"", b"1",
                    lambda _, name: extensions.get(name),
                    lambda _: None, lambda _: None, lambda _: None)
    lib = C.CDLL(str(path))
    entry = Entry.in_dll(lib, "clap_entry")
    assert entry.init(str(path).encode())
    factory_ptr = entry.get_factory(b"clap.plugin-factory")
    factory = C.cast(factory_ptr, C.POINTER(Factory)).contents
    assert factory.count(factory_ptr) == 1
    descriptor = C.cast(factory.descriptor(factory_ptr, 0), C.POINTER(Descriptor)).contents
    assert descriptor.id == b"org.audio-test-reference.shell-experiment"
    ptr = factory.create(factory_ptr, C.byref(host), descriptor.id)
    assert ptr
    plugin = C.cast(ptr, C.POINTER(Plugin)).contents
    assert plugin.init(ptr)
    ports = C.cast(plugin.extension(ptr, b"clap.audio-ports"), C.POINTER(Ports)).contents
    for is_input in [True, False]:
        assert ports.count(ptr, is_input) == 1
        info = PortInfo()
        assert ports.get(ptr, 0, is_input, C.byref(info))
        assert info.channels == 2 and info.type == b"stereo"
        assert not info.flags & 2, "float64 support must not be advertised"
    latency_type = record("Latency", [("get", CB(U, P))])
    latency = C.cast(plugin.extension(ptr, b"clap.latency"), C.POINTER(latency_type)).contents
    for rate in [44_099, 192_001, float("nan"), float("inf")]:
        assert not plugin.activate(ptr, rate, 1, 64), rate
    for rate in [44_100, 48_000, 192_000]:
        assert plugin.activate(ptr, rate, 1, 64)
        assert latency.get(ptr) == 0
        audio_thread = True
        assert plugin.start(ptr)
        events = instance(InputEvents, None, lambda _: 0, lambda *_: None)
        out_events = instance(OutputEvents, None, lambda *_: True)
        # Unequal channels, out-of-unit finite values, signed zero and safety faults.
        left = (F * 8)(0.0, -0.0, 0.125, -2.5, float("nan"), float("inf"), 3.0, 0.0)
        right = (F * 8)(-0.0, 0.5, -0.25, 4.0, 0.75, float("-inf"), -1.0, 0.0)
        expected = [(F * 8)(0.0, -0.0, 0.125, -2.5, 0, 0, 3, 0),
                    (F * 8)(-0.0, 0.5, -0.25, 4, 0.75, 0, -1, 0)]
        # Both separate buffers and host in-place processing.
        for in_place in [False, True]:
            outputs = [left, right] if in_place else [(F * 8)(), (F * 8)()]
            input_ptrs = (C.POINTER(F) * 2)(left, right)
            output_ptrs = (C.POINTER(F) * 2)(*outputs)
            inputs = Audio(input_ptrs, None, 2, 0, 0)
            output = Audio(output_ptrs, None, 2, 0, 0)
            process = Process(0, 8, None, C.pointer(inputs), C.pointer(output), 1, 1,
                              C.pointer(events), C.pointer(out_events))
            assert plugin.process(ptr, C.byref(process)) != 0
            assert [bytes(channel) for channel in outputs] == [bytes(channel) for channel in expected]
        plugin.reset(ptr)
        plugin.stop(ptr)
        audio_thread = False
        plugin.deactivate(ptr)
    if editor_check is not None:
        editor_check(ptr, plugin, flush_requested)
    plugin.destroy(ptr)
    entry.deinit()
    print("PASS: CLAP identity/version, stereo float32 ports, rate rejection, zero latency, finite/safety samples, separate/in-place buffers, lifecycle (no DAW).")
    print(descriptor.name.decode(), descriptor.version_string.decode())


if __name__ == "__main__":
    main()
