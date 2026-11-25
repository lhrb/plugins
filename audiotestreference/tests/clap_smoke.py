#!/usr/bin/env python3
"""Standalone CLAP ABI smoke, adapted from nam-linux-port (GPL-3.0-or-later).
No DAW, editor, user assets, or analyzer. Not DSP certification.
"""
import ctypes as C
from cmath import exp
from math import isclose, pi, sqrt, tan
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
ParamInfo = record("ParamInfo", [("id", U), ("flags", U), ("cookie", P),
                                 ("name", C.c_char * 256), ("module", C.c_char * 1024),
                                 ("min_value", D), ("max_value", D), ("default_value", D)])
Params = record("Params", [("count", CB(U, P)), ("get_info", CB(B, P, U, P)),
                            ("get_value", CB(B, P, U, P)), ("value_to_text", CB(B, P, U, D, P, U)),
                            ("text_to_value", CB(B, P, U, S, P)), ("flush", CB(None, P, P, P))])
EventHeader = record("EventHeader", [("size", U), ("time", U), ("space_id", C.c_uint16),
                                     ("type", C.c_uint16), ("flags", U)])
ParamValue = record("ParamValue", [("header", EventHeader), ("param_id", U), ("cookie", P),
                                   ("note_id", C.c_int32), ("port", C.c_int16),
                                   ("channel", C.c_int16), ("key", C.c_int16), ("value", D)])
CLAP_EVENT_PARAM_VALUE = 5
CLAP_PARAM_IS_STEPPED = 1 << 0
CLAP_PARAM_IS_AUTOMATABLE = 1 << 5


def instance(cls, *values):
    """Wrap callbacks and keep them alive with their owning C structure."""
    return cls(*(typ(value) if isinstance(typ, type) and
                 issubclass(typ, C._CFuncPtr) else value
                 for (_, typ), value in zip(cls._fields_, values)))


def parameter_info(ptr, plugin):
    ext = plugin.extension(ptr, b"clap.params")
    assert ext, "plugin must expose CLAP parameters"
    params = C.cast(ext, C.POINTER(Params)).contents
    infos = {}
    for index in range(params.count(ptr)):
        info = ParamInfo()
        assert params.get_info(ptr, index, C.byref(info)), index
        infos[info.name] = info
    return params, infos


def delay_info(infos):
    names = {b"Delay Time", b"Delay (samples)"} & infos.keys()
    assert len(names) == 1, f"expected one Delay time parameter, got {list(infos)}"
    return infos[names.pop()]


def process_block(ptr, plugin, label, frames, edits):
    # Offset zero only: this adapter applies parameters at block boundaries.
    changes = [ParamValue(EventHeader(C.sizeof(ParamValue), 0, 0, CLAP_EVENT_PARAM_VALUE, 0),
                          param_id, None, -1, -1, -1, -1, value)
               for param_id, value in edits]
    events = instance(InputEvents, None, lambda _: len(changes),
                      lambda _, index: C.addressof(changes[index]))
    out_events = instance(OutputEvents, None, lambda *_: True)
    channel = F * len(frames)
    input_ptrs = (C.POINTER(F) * 2)(*(channel(*samples) for samples in zip(*frames)))
    outputs = [channel(), channel()]
    output_ptrs = (C.POINTER(F) * 2)(*outputs)
    inputs = Audio(input_ptrs, None, 2, 0, 0)
    output = Audio(output_ptrs, None, 2, 0, 0)
    process = Process(-1, len(frames), None, C.pointer(inputs), C.pointer(output), 1, 1,
                      C.pointer(events), C.pointer(out_events))
    assert plugin.process(ptr, C.byref(process)) != 0, label
    return outputs


def check_delay(ptr, plugin, activate, deactivate):
    params, infos = parameter_info(ptr, plugin)
    delay = delay_info(infos)
    mode = infos[b"Mode"]
    assert (mode.min_value, mode.default_value) == (0, 0) and mode.max_value >= 2
    for index, expected in enumerate((b"Identity", b"Gain", b"Delay")):
        text = C.create_string_buffer(64)
        assert params.value_to_text(ptr, mode.id, index, text, len(text)), index
        assert text.value == expected, (index, expected, text.value)
    assert delay.flags & CLAP_PARAM_IS_STEPPED, "Delay time is an integer in samples"
    assert (delay.min_value, delay.max_value, delay.default_value) == (0, 192_000, 0)

    ids = {"mode": mode.id, "delay_samples": delay.id}

    def block(label, frames, expected, **edits):
        outputs = process_block(ptr, plugin, label, frames,
                                [(ids[name], value) for name, value in edits.items()])
        channel = F * len(frames)
        assert [bytes(samples) for samples in outputs] == [bytes(channel(*samples))
                                                         for samples in zip(*expected)], (
            f"{label}: expected {expected}, got {list(zip(*outputs))}")

    silence = (0.0, 0.0)
    a = (0.25, -0.5)
    b = (-0.75, 1.5)

    def fresh_history(label, length, **edits):
        block(label, [b] + [silence] * length, [silence] * length + [b], **edits)

    activate(48_000)
    frame = (-0.0, 0.5)
    block("default zero delay is bit-exact", [frame], [frame], mode=2)
    block("delay exceeds the input block", [a, b], [silence, silence], delay_samples=3)
    block("history crosses into the next block", [silence] * 2, [silence, a])
    block("reapplying unchanged delay keeps history", [silence] * 2, [b, silence], delay_samples=3)

    block("queue samples before changing length", [a, a], [silence] * 2)
    fresh_history("length change drops queued samples", 2, delay_samples=2)
    block("queue sample before leaving Delay", [a], [silence])
    block("Identity is dry even with Delay time set", [b], [b], mode=0)
    fresh_history("returning to Delay drops queued samples", 2, mode=2)
    block("queue sample before reset", [a], [silence])
    plugin.reset(ptr)
    fresh_history("reset clears history but preserves mode and length", 2)
    block("queue sample before reactivation", [a], [silence])
    deactivate()
    activate(44_100)
    fresh_history("reactivation clears history but preserves mode and sample length", 2)
    block("restore defaults for the optional editor check", [a], [a], mode=0, delay_samples=0)
    deactivate()
    print("PASS: CLAP Delay mode/parameter metadata, zero latency, stereo timing and history transitions.")


def check_low_pass(ptr, plugin, activate, deactivate):
    params, infos = parameter_info(ptr, plugin)
    mode = infos[b"Mode"]
    assert mode.max_value >= 3, "Mode must expose Low-pass at index 3"
    assert mode.flags & CLAP_PARAM_IS_STEPPED
    text = C.create_string_buffer(64)
    assert params.value_to_text(ptr, mode.id, 3, text, len(text))
    assert text.value == b"Low-pass", f"Mode 3: expected Low-pass, got {text.value!r}"
    names = (b"Cutoff (fraction)", b"Q")
    assert all(name in infos for name in names), f"expected {names}, got {list(infos)}"
    cutoff, quality = (infos[name] for name in names)
    ids = {"mode": mode, "u": cutoff, "q": quality}
    assert len({info.id for info in ids.values()}) == len(ids)

    def host_value(info, value):
        # NIH-plug exposes continuous CLAP values in [0, 1]. Use the host's
        # numeric text entry to map physical values without assuming a slider curve.
        host = D()
        assert params.text_to_value(ptr, info.id, str(value).encode(), C.byref(host)), (
            f"{info.name!r}: cannot enter {value}")
        return host.value

    def readback(info, expected):
        actual = D()
        assert params.get_value(ptr, info.id, C.byref(actual)), info.name
        assert isclose(actual.value, expected, abs_tol=1e-7, rel_tol=0), (
            f"{info.name!r}: expected CLAP value {expected}, got {actual.value}")

    u, q = 0.1, 1 / sqrt(2)
    for info, lo, hi, default in [(cutoff, 0.0001, 0.49, u), (quality, 0.1, 20, q)]:
        assert (info.min_value, info.max_value) == (0, 1), info.name
        assert not info.flags & CLAP_PARAM_IS_STEPPED, info.name
        assert info.flags & CLAP_PARAM_IS_AUTOMATABLE, info.name
        assert host_value(info, lo) == 0 and host_value(info, hi) == 1, info.name
        assert isclose(info.default_value, host_value(info, default), abs_tol=1e-7, rel_tol=0), (
            f"{info.name!r}: incorrect default {info.default_value}")
        readback(info, info.default_value)

    new_u, new_q = 0.1875, 1.5
    tone_u, tone_q = 0.125, 2.0
    normalized = {(name, value): host_value(ids[name], value)
                  for name, values in [("u", (u, new_u, tone_u)), ("q", (q, new_q, tone_q))]
                  for value in values}

    def block(label, frames, **edits):
        return process_block(ptr, plugin, label, frames, [
            (ids[name].id, value if name == "mode" else normalized[name, value])
            for name, value in edits.items()])

    silence = (0.0, 0.0)
    seed = [(0.25, -0.5), (-0.5, 0.125)]
    zero_history = [(0.0, 0.0, 0.0, 0.0)] * 2
    history = zero_history.copy()

    def compare(label, frames, **edits):
        outputs = block(label, frames, **edits)
        # Bilinear transform of 1/(s² + s/Q + 1), prewarped at u.
        k = tan(pi * u)
        d = 1 + k / q + k * k
        for ch, samples in enumerate(outputs):
            x1, x2, y1, y2 = history[ch]
            for n, (frame, actual) in enumerate(zip(frames, samples)):
                x = frame[ch]
                y = (k*k * (x + 2*x1 + x2) - 2*(k*k - 1)*y1
                     - (1 - k/q + k*k)*y2) / d
                # Exact silence; otherwise 2e-8 + 2e-6*|y| for float32 I/O/host
                # parameter round trips. Signed inputs up to 0.5, including tails.
                tolerance = 0 if y == 0 else 2e-8 + 2e-6 * abs(y)
                assert abs(actual - y) <= tolerance, (
                    f"{label}, channel {ch}, frame {n}: expected {y}, got {actual}")
                x1, x2, y1, y2 = x, x1, y, y1
            history[ch] = (x1, x2, y1, y2)

    def fresh(label, frames=(silence, silence, *seed), **edits):
        history[:] = zero_history
        compare(label, frames, **edits)

    activate(48_000)
    fresh("left impulse and silent right", [(0.5, 0.0)] + [silence] * 7, mode=3)
    plugin.reset(ptr)
    fresh("right impulse and silent left", [(0.0, -0.25)] + [silence] * 7)
    compare("reapplying mode preserves impulse tail across blocks", [silence] * 8 + seed, mode=3)
    u = new_u
    compare("cutoff edit keeps input and output histories", [silence, silence, *seed], u=u)
    q = new_q
    compare("Q edit keeps input and output histories", [silence, silence, *seed], q=q)
    dry = list(zip(*block("leave Low-pass", seed, mode=0)))
    assert dry == seed, f"Identity after Low-pass: expected {seed}, got {dry}"
    fresh("re-entering Low-pass clears history and retains cutoff/Q", mode=3)
    plugin.reset(ptr)
    fresh("lifecycle reset clears history and retains mode/cutoff/Q")
    deactivate()  # Check compensation latency with the Low-pass mode still selected.
    for name, value in [("mode", 3), ("u", normalized["u", u]), ("q", normalized["q", q])]:
        readback(ids[name], value)
    activate(44_100)
    fresh("reactivation at another rate clears history and retains mode/cutoff/Q")

    # 0.125 peak per tone; 512 samples settle, then measure 256 coherent samples.
    # Check complex gain (absolute error <= 3e-6) below, at, and above cutoff.
    size, settle = 256, 512
    bins = (8, 32, 88)
    waves = [[exp(2j*pi*b*n / size) for n in range(size)] for b in bins]
    inputs = [F(0.125 * sum(wave[n].real for wave in waves)).value for n in range(size)]
    outputs = []
    for n in range(0, settle + size, 64):
        frames = [(inputs[i % size], 0.0) for i in range(n, n + 64)]
        edits = {"u": tone_u, "q": tone_q} if n == 0 else {}
        left, _ = block("frequency probe", frames, **edits)
        if n >= settle:
            outputs.extend(left)
    for b, wave in zip(bins, waves):
        measured = (sum(y * w.conjugate() for y, w in zip(outputs, wave)) /
                    sum(x * w.conjugate() for x, w in zip(inputs, wave)))
        ratio = tan(pi*b / size) / tan(pi * tone_u)
        expected = 1 / (1 - ratio**2 + 1j * ratio / tone_q)
        assert abs(measured - expected) <= 3e-6, (
            f"frequency bin {b}/{size}: expected {expected}, got {measured}")
    block("restore defaults for the optional editor check", [silence], mode=0, u=0.1, q=1 / sqrt(2))
    deactivate()
    print("PASS: CLAP Low-pass metadata/readback, stereo impulse and complex response, history and resets.")


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
    def activate(rate):
        nonlocal audio_thread
        assert plugin.activate(ptr, rate, 1, 64)
        assert latency.get(ptr) == 0
        audio_thread = True
        assert plugin.start(ptr)

    def deactivate():
        nonlocal audio_thread
        plugin.stop(ptr)
        audio_thread = False
        assert latency.get(ptr) == 0, "processing must not change host compensation latency"
        plugin.deactivate(ptr)

    for rate in [44_100, 48_000, 192_000]:
        activate(rate)
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
        deactivate()
    check_delay(ptr, plugin, activate, deactivate)
    check_low_pass(ptr, plugin, activate, deactivate)
    if editor_check is not None:
        editor_check(ptr, plugin, flush_requested)
    plugin.destroy(ptr)
    entry.deinit()
    print("PASS: CLAP identity/version, stereo float32 ports, rate rejection, zero latency, finite/safety samples, separate/in-place buffers, lifecycle (no DAW).")
    print(descriptor.name.decode(), descriptor.version_string.decode())


if __name__ == "__main__":
    main()
