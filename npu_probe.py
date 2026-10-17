#!/usr/bin/env python3
"""Read the amdxdna NPU query ABI and print what the driver reports.

Run on the host, as a user that can open /dev/accel/accel0:

    python3 npu_probe.py

No dependencies beyond the standard library. The struct layouts and query
ids come from the running kernel's BTF (/sys/kernel/btf/amdxdna) and
include/uapi/drm/amdxdna_accel.h.
"""

import ctypes
import errno
import os
import struct
import sys

ACCEL = "/dev/accel/accel0"
VERSION = 2

# DRM_IOWR(DRM_COMMAND_BASE + DRM_AMDXDNA_GET_INFO, struct drm_amdxdna_get_info)
# DRM_COMMAND_BASE = 0x40, GET_INFO = 7, sizeof(struct) = 16.
# The _IOW variant is included because some kernels use it instead.
REQS = [
    0xC0106447,  # _IOWR('d', 0x47, 16) - the real one
    0x40106447,  # _IOW ('d', 0x47, 16) - some kernels use this direction
]

QUERY = {
    0: "AIE_STATUS",
    1: "AIE_METADATA",
    2: "AIE_VERSION",
    3: "CLOCK_METADATA",
    4: "SENSORS",
    5: "HW_CONTEXTS",
    8: "FIRMWARE_VERSION",
    9: "POWER_MODE",
    10: "TELEMETRY",
    12: "RESOURCE_INFO",
}

libc = ctypes.CDLL(None, use_errno=True)
libc.ioctl.restype = ctypes.c_int
libc.ioctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_void_p]


class GetInfo(ctypes.Structure):
    _fields_ = [
        ("param", ctypes.c_uint32),
        ("buffer_size", ctypes.c_uint32),
        ("buffer", ctypes.c_uint64),
    ]


def cstr(raw):
    return raw.split(b"\0", 1)[0].decode("utf-8", "replace")


def query(fd, param, size):
    """Call GET_INFO. Returns (getinfo, result_bytes) or raises OSError."""
    buf = ctypes.create_string_buffer(size)
    info = GetInfo(param, size, ctypes.addressof(buf))
    last = 0
    for req in REQS:
        ctypes.set_errno(0)
        rc = libc.ioctl(fd, req, ctypes.byref(info))
        if rc == 0:
            return info, buf.raw
        last = ctypes.get_errno()
        print(f"  [query param {param}: req {req:#x} -> {errno.errorcode.get(last, last)}]")
        if last != errno.ENOTTY:
            break
    raise OSError(last, os.strerror(last))


def show_sensors(fd):
    info, data = query(fd, 4, 168 * 32)
    print(f"\nSENSORS (buffer_size returned {info.buffer_size})")
    n = 0
    for off in range(0, len(data) - 168 + 1, 168):
        label = cstr(data[off : off + 64])
        stype = data[off + 161]
        inp, mx, avg, hi = struct.unpack_from("<IIII", data, off + 64)
        if not label and not (inp or mx or avg or hi):
            continue
        units = cstr(data[off + 144 : off + 160])
        unitm = struct.unpack_from("<b", data, off + 160)[0]
        status = cstr(data[off + 80 : off + 144])
        scale = 10.0 ** unitm
        print(
            f"  [{stype}] {label!r}: input={inp} max={mx} avg={avg} highest={hi} "
            f"units={units!r} unitm={unitm:+d} -> {inp * scale:g} {units} "
            f"status={status!r}"
        )
        n += 1
    if not n:
        print("  no sensors reported")


def show_metadata(fd):
    info, data = query(fd, 1, 64)
    col_size, cols, rows = struct.unpack_from("<IHH", data, 0)
    vmaj, vmin = struct.unpack_from("<II", data, 8)
    print(f"\nAIE_METADATA: col_size={col_size} cols={cols} rows={rows} aie_version={vmaj}.{vmin}")
    for name, off in (("core", 16), ("mem", 32), ("shim", 48)):
        rc, rs, dma, locks, events = struct.unpack_from("<5H", data, off)
        print(f"  {name:<4} rows={rc} row_start={rs} dma={dma} locks={locks} events={events}")


def show_clocks(fd):
    info, data = query(fd, 3, 48)
    print("\nCLOCK_METADATA:")
    for off in (0, 24):
        name = cstr(data[off : off + 16])
        freq = struct.unpack_from("<I", data, off + 16)[0]
        print(f"  {name!r}: {freq} MHz")


def show_fw(fd):
    info, data = query(fd, 8, 16)
    major, minor, patch, build = struct.unpack_from("<IIII", data, 0)
    print(f"\nFIRMWARE_VERSION: {major}.{minor}.{patch}.{build}")


def show_power_mode(fd):
    info, data = query(fd, 9, 8)
    mode = data[0]
    names = ["default", "low", "medium", "high", "turbo"]
    print(f"\nPOWER_MODE: {mode} ({names[mode] if mode < len(names) else '?'})")


def show_resource(fd):
    info, data = query(fd, 12, 40)
    clk, tops_max, task_max, tops_cur, task_cur = struct.unpack_from("<QQQQQ", data, 0)
    print(
        f"\nRESOURCE_INFO: npu_clk_max={clk} MHz tops_max={tops_max} task_max={task_max} "
        f"tops_curr={tops_cur} task_curr={task_cur}"
    )


def show_contexts(fd):
    info, data = query(fd, 5, 144 * 64)
    print(f"\nHW_CONTEXTS (buffer_size returned {info.buffer_size})")
    n = 0
    for off in range(0, len(data) - 144 + 1, 144):
        ctx, start, ncol, hwctx, pid = struct.unpack_from("<IIIIq", data, off)
        if ctx == 0 and pid == 0:
            continue
        (subs, comps, mig, preempt, errs, prio, heap, susp) = struct.unpack_from(
            "<8Q", data, off + 24
        )
        state, pasid, gops, fps, dma, lat, frame, txn, pc = struct.unpack_from(
            "<9I", data, off + 88
        )
        print(
            f"  ctx={ctx} hwctx={hwctx} pid={pid} cols={start}+{ncol} state={state} "
            f"pasid={pasid} heap={heap} subs={subs} comps={comps} errs={errs} "
            f"preempt={preempt} mig={mig} susp={susp} prio={prio}"
        )
        print(
            f"    gops={gops} fps={fps} dma_bandwidth={dma} latency={lat} "
            f"frame_exec={frame} txn={txn} pc={pc}"
        )
        n += 1
    if not n:
        print("  no active contexts")


def show_aie_status(fd):
    data_buf = ctypes.create_string_buffer(4096)
    status = ctypes.create_string_buffer(16)
    struct.pack_into("<QII", status, 0, ctypes.addressof(data_buf), 4096, 0)
    info = GetInfo(0, 16, ctypes.addressof(status))
    last = 0
    for req in REQS:
        ctypes.set_errno(0)
        if libc.ioctl(fd, req, ctypes.byref(info)) == 0:
            break
        last = ctypes.get_errno()
        if last != errno.ENOTTY:
            break
    else:
        raise OSError(last, os.strerror(last))
    ptr, size, cols_filled = struct.unpack_from("<QII", status.raw, 0)
    print(f"\nAIE_STATUS: cols_filled={cols_filled} buffer={ptr:#x} size={size}")
    if ptr:
        raw = ctypes.string_at(ptr, min(size, 256))
        print("  data[0:64] =", raw[:64].hex(" "))


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else ACCEL
    try:
        fd = os.open(path, os.O_RDWR)
    except OSError as e:
        if e.errno == errno.EACCES:
            fd = os.open(path, os.O_RDONLY)
        else:
            print(f"cannot open {path}: {e}")
            print("run this on the host, not inside the container")
            return 1
    print(f"npu_probe v{VERSION}")
    print(f"opened {path} (fd {fd})")
    failures = []
    for fn in (
        show_metadata,
        show_clocks,
        show_fw,
        show_power_mode,
        show_resource,
        show_sensors,
        show_contexts,
        show_aie_status,
    ):
        try:
            fn(fd)
        except OSError as e:
            failures.append((fn.__name__, e))
            print(f"\n{fn.__name__}: {e.strerror} (errno {e.errno})")
    os.close(fd)
    if failures:
        print("\nfailed queries:", ", ".join(f"{n} ({e.errno})" for n, e in failures))
    return 0


if __name__ == "__main__":
    sys.exit(main())
