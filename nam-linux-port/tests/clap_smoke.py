#!/usr/bin/env python3
"""Real CLAP ABI smoke test; stdlib only, no editor/display required.

Run after `cargo build --no-default-features --lib`:
    python3 tests/clap_smoke.py target/debug/libnam_linux_port.so

Tests default mono->stereo audio, normalized CLAP automation, NIH state recall,
and official NAM inference using a temporary Linear model. Never loads user assets.
"""
import argparse
import ctypes as C
import json
import math
from pathlib import Path
import struct
import tempfile
import threading

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
HostParams = record("HostParams", [("rescan", CB(None, P, U)),
                                   ("clear", CB(None, P, U, U)),
                                   ("request_flush", CB(None, P))])
Header = record("Header", [("size", U), ("time", U), ("space", C.c_uint16),
                           ("type", C.c_uint16), ("flags", U)])
ParamEvent = record("ParamEvent", [("header", Header), ("id", U), ("cookie", P),
                                   ("note", C.c_int32), ("port", C.c_int16),
                                   ("channel", C.c_int16), ("key", C.c_int16), ("value", D)])
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
ParamInfo = record("ParamInfo", [("id", U), ("flags", U), ("cookie", P),
                                 ("name", C.c_char * 256), ("module", C.c_char * 1024),
                                 ("min", D), ("max", D), ("default", D)])
Params = record("Params", [("count", CB(U, P)), ("info", CB(B, P, U, P)),
                           ("value", CB(B, P, U, P)), ("to_text", P), ("from_text", P),
                           ("flush", CB(None, P, P, P))])
PortInfo = record("PortInfo", [("id", U), ("name", C.c_char * 256), ("flags", U),
                               ("channels", U), ("type", S), ("pair", U)])
Ports = record("Ports", [("count", CB(U, P, B)), ("get", CB(B, P, U, B, P))])
State = record("State", [("save", CB(B, P, P)), ("load", CB(B, P, P))])
OStream = record("OStream", [("ctx", P), ("write", CB(C.c_int64, P, P, C.c_uint64))])
IStream = record("IStream", [("ctx", P), ("read", CB(C.c_int64, P, P, C.c_uint64))])


def instance(cls, *values):
    """Wrap callbacks and keep them alive with their owning C structure."""
    return cls(*(typ(value) if isinstance(typ, type) and
                 issubclass(typ, C._CFuncPtr) else value
                 for (_, typ), value in zip(cls._fields_, values)))


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def param_id(text):
    value = 0
    for byte in text.encode():
        value = (value * 31 + byte) & 0xFFFFFFFF
    return value & 0x7FFFFFFF


# NIH exports continuous parameters normalized to [0,1], bools as [0,1].
DEFAULTS = dict(input=0.5, threshold=0.2, bass=0.5, middle=0.5, treble=0.5,
                output=0.5, gate_enabled=1.0, eq_enabled=1.0, ir_enabled=1.0)
FRAMES = 64
SIGNAL = [0.125, -0.25, 0.0625, -0.03125] * (FRAMES // 4)


class Smoke:
    def __init__(self, path):
        self.main_id = threading.get_ident()
        self.audio_thread = False
        self.pending = threading.Event()
        self.restart_requested = False
        self.thread_check = instance(ThreadCheck,
                                     lambda _: threading.get_ident() == self.main_id and not self.audio_thread,
                                     lambda _: self.audio_thread)
        self.host_params = instance(HostParams, lambda *_: None, lambda *_: None, lambda _: None)
        self.host = instance(Host, Version(1, 2, 0), None, b"Python CLAP smoke",
                             b"NAM Linux tests", b"", b"1",
                             lambda _, name: C.addressof(self.thread_check)
                             if name == b"clap.thread-check" else
                             C.addressof(self.host_params) if name == b"clap.params" else None,
                             self.request_restart, lambda _: None, lambda _: self.pending.set())
        self.lib = C.CDLL(str(path))
        self.entry = Entry.in_dll(self.lib, "clap_entry")
        self.plugin = None
        self.active = self.started = self.initialized = False
        require(self.entry.version.major == 1, "CLAP entry major version must be 1")
        require(self.entry.init(str(path).encode()), "clap_entry.init failed")
        self.initialized = True
        factory_ptr = self.entry.get_factory(b"clap.plugin-factory")
        require(factory_ptr, "missing CLAP plugin factory")
        factory = C.cast(factory_ptr, C.POINTER(Factory)).contents
        require(factory.count(factory_ptr) == 1, "expected one NAM plugin")
        descriptor_ptr = factory.descriptor(factory_ptr, 0)
        require(descriptor_ptr, "missing plugin descriptor")
        descriptor = C.cast(descriptor_ptr, C.POINTER(Descriptor)).contents
        require(descriptor.id == b"org.nam-linux.amplifier", f"unexpected plugin ID: {descriptor.id!r}")
        self.ptr = factory.create(factory_ptr, C.byref(self.host), descriptor.id)
        require(self.ptr, "factory.create failed")
        self.plugin = C.cast(self.ptr, C.POINTER(Plugin)).contents
        require(self.plugin.init(self.ptr), "plugin.init failed")
        self.params = self.extension(b"clap.params", Params)
        self.state = self.extension(b"clap.state", State)
        self.pump()

    def request_restart(self, _):
        self.restart_requested = True

    def extension(self, name, cls):
        ptr = self.plugin.extension(self.ptr, name)
        require(ptr, f"missing {name.decode()} extension")
        return C.cast(ptr, C.POINTER(cls)).contents

    def pump(self):
        for _ in range(100):
            if not self.pending.is_set():
                return
            self.pending.clear()
            self.plugin.on_main_thread(self.ptr)
        raise AssertionError("host request_callback did not quiesce after 100 callbacks")

    def value(self, name):
        value = D()
        require(self.params.value(self.ptr, param_id(name), C.byref(value)), f"get_value({name}) failed")
        return value.value

    def check_params(self):
        require(self.params.count(self.ptr) == 9, "expected exactly nine parameters")
        seen = set()
        expected = {param_id(name): (name, value) for name, value in DEFAULTS.items()}
        for index in range(9):
            info = ParamInfo()
            require(self.params.info(self.ptr, index, C.byref(info)), f"get_info({index}) failed")
            require(info.id in expected and info.id not in seen, f"unexpected/duplicate parameter ID {info.id}")
            seen.add(info.id)
            name, default = expected[info.id]
            require(info.min == 0 and info.max == 1, f"{name}: expected normalized [0,1] range")
            require(math.isclose(info.default, default, abs_tol=1e-6), f"{name}: wrong default {info.default}")
            require(math.isclose(self.value(name), default, abs_tol=1e-6), f"{name}: initial value differs from default")
        ports = self.extension(b"clap.audio-ports", Ports)
        for is_input, channels in ((True, 1), (False, 2)):
            require(ports.count(self.ptr, is_input) == 1, "expected one main port in each direction")
            info = PortInfo()
            require(ports.get(self.ptr, 0, is_input, C.byref(info)), "audio port get failed")
            require(info.channels == channels, f"default {'input' if is_input else 'output'} must have {channels} channels")
        print("PASS: factory, init, nine stable parameter IDs/defaults, mono->stereo ports")

    def activate(self):
        require(self.plugin.activate(self.ptr, 48000.0, 1, FRAMES), "activate failed")
        self.active = True
        self.audio_thread = True
        try:
            require(self.plugin.start(self.ptr), "start_processing failed")
            self.started = True
        finally:
            self.audio_thread = False

    def deactivate(self):
        if self.started:
            self.audio_thread = True
            try:
                self.plugin.stop(self.ptr)
            finally:
                self.audio_thread = False
            self.started = False
        if self.active:
            self.plugin.deactivate(self.ptr)
            self.active = False
        self.pump()
        self.restart_requested = False

    def process(self, changes=()):
        events = [ParamEvent(Header(C.sizeof(ParamEvent), 0, 0, 5, 0), param_id(name),
                             None, -1, -1, -1, -1, value) for name, value in changes]
        inputs = instance(InputEvents, None, lambda _: len(events),
                          lambda _, index: C.addressof(events[index]) if index < len(events) else None)
        # Host automation need not be echoed by a plugin. Accept any valid output
        # events rather than asserting a spurious echo contract.
        output_sizes = []
        def push(_, ptr):
            header = C.cast(ptr, C.POINTER(Header)).contents
            output_sizes.append((header.size, header.time))
            return True
        outputs = instance(OutputEvents, None, push)
        source = (F * FRAMES)(*SIGNAL)
        left, right = (F * FRAMES)(*[float("nan")] * FRAMES), (F * FRAMES)(*[float("nan")] * FRAMES)
        in_ptrs = (C.POINTER(F) * 1)(source)
        out_ptrs = (C.POINTER(F) * 2)(left, right)
        audio_in, audio_out = Audio(in_ptrs, None, 1, 0, 0), Audio(out_ptrs, None, 2, 0, 0)
        block = Process(-1, FRAMES, None, C.pointer(audio_in), C.pointer(audio_out),
                        1, 1, C.pointer(inputs), C.pointer(outputs))
        self.audio_thread = True
        try:
            status = self.plugin.process(self.ptr, C.byref(block))
        finally:
            self.audio_thread = False
        require(status in (1, 2, 3, 4), f"process returned error/unknown status {status}")
        require(all(math.isfinite(x) for channel in (left, right) for x in channel), "process left non-finite/unwritten output")
        require(all(size >= C.sizeof(Header) and frame < FRAMES for size, frame in output_sizes), "invalid output event header")
        self.pump()
        return [list(left), list(right)]

    def expect_audio(self, gain, label, settle=False):
        # Gain automation deliberately ramps over 5ms. Render a fixed 85ms of
        # audio when checking settled controls; never sleep or poll for assets.
        if settle:
            for _ in range(64):
                self.process()
        channels = self.process()
        error = max(abs(actual - sample * gain) for channel in channels
                    for actual, sample in zip(channel, SIGNAL))
        require(error < 2e-5,
                f"{label}: expected both channels = input * {gain:g}; max error {error:g}; first samples {[c[:4] for c in channels]}")

    def save(self):
        data = bytearray()
        def write(_, ptr, count):
            # Exercise valid short writes, including the length prefix.
            count = min(count, 17)
            data.extend(C.string_at(ptr, count))
            return count
        stream = instance(OStream, None, write)
        require(self.state.save(self.ptr, C.byref(stream)), "state.save failed")
        return bytes(data)

    def load(self, data):
        require(not self.active, "test restores state only while inactive")
        offset = 0
        def read(_, ptr, count):
            nonlocal offset
            chunk = data[offset:offset + min(count, 13)]
            C.memmove(ptr, chunk, len(chunk))
            offset += len(chunk)
            return len(chunk)
        stream = instance(IStream, None, read)
        require(self.state.load(self.ptr, C.byref(stream)), "state.load failed")
        require(offset == len(data), "state.load did not consume full state")
        self.pump()

    def close(self):
        if self.plugin is not None:
            self.deactivate()
            self.plugin.destroy(self.ptr)
            self.plugin = None
        if self.initialized:
            self.entry.deinit()
            self.initialized = False


def decode_state(data):
    require(len(data) >= 8, "NIH state missing length prefix")
    size, = struct.unpack("<Q", data[:8])
    require(size == len(data) - 8, "NIH state length prefix mismatch")
    return json.loads(data[8:])


def encode_state(state):
    payload = json.dumps(state).encode()
    return struct.pack("<Q", len(payload)) + payload


def run(smoke):
    smoke.check_params()
    smoke.activate()
    smoke.process((("gate_enabled", 0), ("eq_enabled", 0)))
    smoke.expect_audio(1.0, "neutral passthrough")
    smoke.process((("input", 0.625), ("output", 0.375)))  # +6 dB / -6 dB
    smoke.expect_audio(1.0, "input/output automation cancellation", settle=True)
    smoke.process((("input", 0.5),))
    smoke.expect_audio(10 ** (-6 / 20), "output gain automation", settle=True)
    require(smoke.value("gate_enabled") == 0 and smoke.value("eq_enabled") == 0,
            "automation did not disable gate/EQ")
    require(math.isclose(smoke.value("output"), 0.375, abs_tol=1e-6), "output automation value not retained")
    print("PASS: finite neutral audio, gate/EQ disable, input/output gain automation")
    smoke.deactivate()
    saved = smoke.save()
    smoke.activate()
    smoke.process((("output", 1.0), ("gate_enabled", 1)))
    smoke.deactivate()
    smoke.load(saved)
    require(math.isclose(smoke.value("output"), 0.375, abs_tol=1e-6), "state did not restore output parameter")
    require(smoke.value("gate_enabled") == 0, "state did not restore gate parameter")
    smoke.activate()
    smoke.expect_audio(10 ** (-6 / 20), "restored parameter audio", settle=True)
    smoke.process((("output", 0.5),))
    smoke.expect_audio(1.0, "neutral gain before NAM recall", settle=True)
    smoke.deactivate()
    saved = smoke.save()
    print("PASS: CLAP short-read/write state streams restore parameters and audio")
    with tempfile.TemporaryDirectory(prefix="nam-clap-smoke-") as directory:
        model = Path(directory) / "linear ü.nam"
        model.write_text(json.dumps(dict(version="0.5.4", architecture="Linear",
                                        config=dict(receptive_field=1, bias=False, in_channels=1, out_channels=1),
                                        weights=[0.25], sample_rate=48000)), encoding="utf-8")
        state = decode_state(saved)
        require("assets" in state["fields"], "NIH state lacks persisted assets field")
        assets = json.loads(state["fields"]["assets"])
        require(assets == {"model": None, "ir": None}, f"unexpected initial assets {assets!r}")
        assets["model"] = str(model)
        state["fields"]["assets"] = json.dumps(assets)
        smoke.load(encode_state(state))
        persisted_bytes = smoke.save()
        persisted = decode_state(persisted_bytes)
        require(json.loads(persisted["fields"]["assets"]) == assets, "state did not persist NAM model path")
        # Round-trip the plugin's own saved bytes, not only our modified JSON.
        smoke.load(persisted_bytes)
        smoke.activate()
        # This must be the FIRST block after activation, at neutral gain: no
        # worker polling or smoothing can hide a silent/stale initial model.
        smoke.expect_audio(0.25, "first-block restored Linear NAM inference")
        smoke.deactivate()
        print("PASS: persisted Unicode .nam path, state round trip, real NAM Linear inference on both outputs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("library", type=Path, help="path to libnam_linux_port.so or NAM .clap")
    args = parser.parse_args()
    path = args.library.resolve()
    require(path.is_file(), f"plugin library not found: {path}; build --no-default-features --lib first")
    smoke = None
    try:
        smoke = Smoke(path)
        run(smoke)
    finally:
        if smoke is not None:
            smoke.close()
    print("PASS: CLAP smoke completed")


if __name__ == "__main__":
    main()
