#!/usr/bin/env python3
"""Minimal amdxdna context smoke test: allocate the NPU array for N seconds.

Creates a DEV_HEAP buffer object, then a hardware context covering `--tiles`
AIE core tiles (num_tiles / core_row_count = columns), holds it, and polls the
driver's own HW_CONTEXTS / AIE_STATUS queries so you can watch amdtop react.

Needs read/write access to /dev/accel/accelN (render group).
This pokes the uapi directly; it is a test helper, not part of amdtop.
"""

import argparse
import ctypes
import fcntl
import os
import sys
import time

DRM_COMMAND_BASE = 0x40
DRM_AMDXDNA_CREATE_HWCTX = DRM_COMMAND_BASE + 0
DRM_AMDXDNA_DESTROY_HWCTX = DRM_COMMAND_BASE + 1
DRM_AMDXDNA_CREATE_BO = DRM_COMMAND_BASE + 3
DRM_AMDXDNA_GET_BO_INFO = DRM_COMMAND_BASE + 4
DRM_AMDXDNA_GET_INFO = DRM_COMMAND_BASE + 7

# enum amdxdna_drm_get_param
QUERY_AIE_STATUS = 0
QUERY_HW_CONTEXTS = 5

# enum amdxdna_bo_type
BO_SHMEM, BO_DEV_HEAP, BO_DEV, BO_CMD = 1, 2, 3, 4

DEVM_SIZE = 64 << 20          # AIE2_DEVM_SIZE: heap BO must be a multiple
QOS_NORMAL = 0x200


def iowr(nr, size):
    return (3 << 30) | (size << 16) | (ord("d") << 8) | nr


class CreateBo(ctypes.Structure):
    _fields_ = [
        ("flags", ctypes.c_uint64),
        ("vaddr", ctypes.c_uint64),
        ("size", ctypes.c_uint64),
        ("type", ctypes.c_uint32),
        ("handle", ctypes.c_uint32),
    ]


class Qos(ctypes.Structure):
    _fields_ = [
        ("gops", ctypes.c_uint32),
        ("fps", ctypes.c_uint32),
        ("dma_bandwidth", ctypes.c_uint32),
        ("latency", ctypes.c_uint32),
        ("frame_exec_time", ctypes.c_uint32),
        ("priority", ctypes.c_uint32),
    ]


class CreateHwctx(ctypes.Structure):
    _fields_ = [
        ("ext", ctypes.c_uint64),
        ("ext_flags", ctypes.c_uint64),
        ("qos_p", ctypes.c_uint64),
        ("umq_bo", ctypes.c_uint32),
        ("log_buf_bo", ctypes.c_uint32),
        ("max_opc", ctypes.c_uint32),
        ("num_tiles", ctypes.c_uint32),
        ("mem_size", ctypes.c_uint32),
        ("umq_doorbell", ctypes.c_uint32),
        ("handle", ctypes.c_uint32),
        ("syncobj_handle", ctypes.c_uint32),
    ]


class DestroyHwctx(ctypes.Structure):
    _fields_ = [("handle", ctypes.c_uint32), ("pad", ctypes.c_uint32)]


class GetBoInfo(ctypes.Structure):
    _fields_ = [
        ("ext", ctypes.c_uint64),
        ("ext_flags", ctypes.c_uint64),
        ("handle", ctypes.c_uint32),
        ("pad", ctypes.c_uint32),
        ("map_offset", ctypes.c_uint64),
        ("vaddr", ctypes.c_uint64),
        ("xdna_addr", ctypes.c_uint64),
    ]


class GetInfo(ctypes.Structure):
    _fields_ = [
        ("param", ctypes.c_uint32),
        ("buffer_size", ctypes.c_uint32),
        ("buffer", ctypes.c_uint64),
    ]


class AieStatus(ctypes.Structure):
    _fields_ = [
        ("buffer", ctypes.c_uint64),
        ("buffer_size", ctypes.c_uint32),
        ("cols_filled", ctypes.c_uint32),
    ]


class HwCtx(ctypes.Structure):
    _fields_ = [
        ("context_id", ctypes.c_uint32),
        ("start_col", ctypes.c_uint32),
        ("num_col", ctypes.c_uint32),
        ("pad", ctypes.c_uint32),
        ("pid", ctypes.c_int64),
        ("command_submissions", ctypes.c_uint64),
        ("command_completions", ctypes.c_uint64),
        ("migrations", ctypes.c_uint64),
        ("preemptions", ctypes.c_uint64),
        ("errors", ctypes.c_uint64),
    ]


def ioctl(fd, nr, struct):
    return fcntl.ioctl(fd, iowr(nr, ctypes.sizeof(struct)), struct)


def query(fd, param, buf):
    info = GetInfo(param, ctypes.sizeof(buf), ctypes.addressof(buf))
    ioctl(fd, DRM_AMDXDNA_GET_INFO, info)
    return info.buffer_size


def show(fd):
    raw = (ctypes.c_uint8 * (64 * ctypes.sizeof(HwCtx)))()
    query(fd, QUERY_HW_CONTEXTS, raw)
    hw = ctypes.cast(raw, ctypes.POINTER(HwCtx))
    live = [c for c in hw[:64] if c.context_id or c.pid]
    count = len(live)
    parts = [
        f"pid {c.pid} ctx {c.context_id} cols {c.num_col}@{c.start_col}"
        for c in live
    ]
    status = (ctypes.c_uint8 * 4096)()
    aie = AieStatus(ctypes.addressof(status), ctypes.sizeof(status), 0)
    query(fd, QUERY_AIE_STATUS, aie)
    print(
        f"  contexts {count}: " + ("; ".join(parts) if parts else "none")
        + f" | cols_filled 0x{aie.cols_filled:02x} ({bin(aie.cols_filled & 0xFF).count('1')} cols)"
    )


class VaEntry(ctypes.Structure):
    _fields_ = [("vaddr", ctypes.c_uint64), ("len", ctypes.c_uint64)]


class VaTbl(ctypes.Structure):
    _fields_ = [("dmabuf_fd", ctypes.c_int32), ("num_entries", ctypes.c_uint32)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dev", default="/dev/accel/accel0")
    ap.add_argument("--tiles", type=int, default=32, help="AIE core tiles (32 = all 8 columns)")
    ap.add_argument("--hold", type=float, default=20.0, help="seconds to keep the context")
    ap.add_argument("--heap-mb", type=int, default=8, help="dev heap size in MiB")
    ap.add_argument("--mmap-heap", action="store_true", default=True, help="mmap the heap BO before hwctx")
    ap.add_argument("--heap", default="none", choices=["table", "raw", "none"],
                    help="how to describe the dev-heap backing buffer")
    args = ap.parse_args()
    heap_size = args.heap_mb << 20

    # The dev heap is backed by a user mapping the driver pins, described by a
    # va_table (dmabuf_fd = -1, one {vaddr, len} entry).
    libc = ctypes.CDLL(None, use_errno=True)
    libc.mmap.restype = ctypes.c_void_p
    libc.mmap.argtypes = [
        ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_long,
    ]
    align = 2 << 20
    base = libc.mmap(None, heap_size + align, 3, 0x22, -1, 0)  # PROT_RW|MAP_PRIVATE|ANON
    if base in (None, ctypes.c_void_p(-1).value) or (base + 1) == 0:
        raise OSError(ctypes.get_errno(), "mmap failed")
    userptr = (base + align - 1) & ~(align - 1)

    tbl = (ctypes.c_uint8 * (ctypes.sizeof(VaTbl) + ctypes.sizeof(VaEntry)))()
    vt = VaTbl.from_buffer(tbl)
    vt.dmabuf_fd = -1
    vt.num_entries = 1
    entry = VaEntry.from_buffer(tbl, ctypes.sizeof(VaTbl))
    entry.vaddr = userptr
    entry.len = heap_size

    # Fault the range in so the driver can pin it.
    ctypes.memset(userptr, 0, heap_size)

    vaddr = {"table": ctypes.addressof(tbl), "raw": userptr, "none": 0}[args.heap]
    print(f"heap backing: {args.heap} vaddr {vaddr:#x} userptr {userptr:#x}")

    fd = os.open(args.dev, os.O_RDWR)
    try:
        bo = CreateBo(0, vaddr, heap_size, BO_DEV_HEAP, 0)
        ioctl(fd, DRM_AMDXDNA_CREATE_BO, bo)
        print(f"heap BO handle {bo.handle} size {heap_size >> 20} MiB")

        if args.mmap_heap:
            info = GetBoInfo(0, 0, bo.handle, 0, 0, 0, 0)
            ioctl(fd, DRM_AMDXDNA_GET_BO_INFO, info)
            mapped = libc.mmap(None, heap_size, 3, 1, fd, info.map_offset)  # MAP_SHARED
            print(f"heap mmap off {info.map_offset:#x} -> {mapped:#x} uva {info.vaddr:#x}")

        qos = Qos(0, 0, 0, 0, 0, QOS_NORMAL)
        # The firmware can briefly refuse while it tears down a previous client.
        for attempt in range(6):
            hw = CreateHwctx(0, 0, ctypes.addressof(qos), 0, 0, 0, args.tiles, 0, 0, 0, 0)
            try:
                ioctl(fd, DRM_AMDXDNA_CREATE_HWCTX, hw)
                break
            except OSError as exc:
                if exc.errno != 22 or attempt == 5:
                    raise
                print(f"  create refused (attempt {attempt + 1}), retrying")
                time.sleep(1.0)
        print(f"hwctx handle {hw.handle}, {args.tiles} tiles -> {args.tiles // 4} columns")
        show(fd)

        deadline = time.time() + args.hold
        while time.time() < deadline:
            time.sleep(3)
            show(fd)

        ioctl(fd, DRM_AMDXDNA_DESTROY_HWCTX, DestroyHwctx(hw.handle, 0))
        print("context destroyed")
    finally:
        os.close(fd)


if __name__ == "__main__":
    try:
        main()
    except OSError as exc:
        print(f"ioctl failed: {exc}", file=sys.stderr)
        sys.exit(1)
