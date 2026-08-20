#!/usr/bin/env python3
"""Reassemble a chunked preset read-stream out of a USBPcap capture (read-only, no deps).

A preset is far bigger than a bulk packet, so the device answers a read with one logical stream
split across many `cmd 0x04` frames on the edit channel (`dev.EDIT -> host.EDIT`), interleaved
with its own empty `cmd 0x08` flow-control frames. The first data frame carries an 8-byte stream
header followed by the MessagePack envelope `{102: counter, 103: 0, 104: <str16/32 blob>}`; the
`104` marker declares the blob length, and the continuation frames are raw payload bytes.

We reassemble by that declared length rather than by the header — the header's type field varies
per stream (it tracks the low byte of the blob length), so it is not a reliable total.

Output is byte-for-byte the shape of the tracked `captures/*.msgpack.bin` fixtures (8-byte header
included), so it feeds straight into `fretwire_data::stream::PresetStream::parse`.

With --list it reassembles **preset-list** streams instead: same envelope, but key 104 holds the
array of per-slot entries rather than a nested blob, and HX Edit runs the browse on the primary
channel rather than the edit one.

Usage:
  tools/extract-preset-stream.py <cap.pcapng> [out-prefix] [--min BYTES] [--list]

Self-check: run it on `captures/open_two_presets_one_after_another.pcapng` and the first stream
should be byte-identical to `captures/preset1_stream.msgpack.bin`.

Companion to pcap-frames.py (raw frames) and decode-edits.py (envelopes). Reassembly used to be
done by hand — see captures/open_two_presets.md.
"""
import struct, sys

EDIT_DEV = 0x03ed  # dev.EDIT -> host.EDIT
PRI_DEV = 0x03ef   # dev.PRI  -> host.PRI, where HX Edit runs the preset browse
STREAM_PREFIX = 8  # bytes before the envelope: u32 LE total at offset 4


def walk(buf):
    off, n = 0, len(buf)
    while off + 12 <= n:
        bt, bl = struct.unpack_from("<II", buf, off)
        if bl < 12 or off + bl > n:
            break
        if bt == 6:
            cap = struct.unpack_from("<I", buf, off + 20)[0]
            yield buf[off + 28:off + 28 + cap]
        off += bl


def usb(pkt):
    if len(pkt) < 27:
        return None
    hl = struct.unpack_from("<H", pkt, 0)[0]
    if hl < 27 or hl > len(pkt):
        return None
    return pkt[21], pkt[22], pkt[hl:]


def decode_frame(d):
    if len(d) < 16 or d[3] not in (0x18, 0x28):
        return None
    ln = d[0] | (d[1] << 8)
    return dict(src=d[4] | (d[5] << 8), cmd=d[11],
                body=bytes(d[16:16 + max(0, ln - 8)]))


def stream_total(b):
    """If `b` opens a preset stream, return its total reassembled length, else None.

    Layout: 8-byte header, then msgpack `83 66 <counter> 67 <n> 68 <str marker> <blob>`.
    Keys: 102 (`0x66`) counter, 103 (`0x67`), 104 (`0x68`) the blob.
    """
    if len(b) < 16 or b[8] != 0x83 or b[9] != 0x66:
        return None
    i = 10
    # counter: positive fixint, uint8, uint16 or uint32
    m = b[i]
    i += 1 if m < 0x80 else {0xcc: 2, 0xcd: 3, 0xce: 5}.get(m, 0)
    if i <= 10 or i + 2 > len(b) or b[i] != 0x67:
        return None
    i += 1
    m = b[i]
    i += 1 if m < 0x80 else {0xcc: 2, 0xcd: 3, 0xce: 5}.get(m, 0)
    if i + 2 > len(b) or b[i] != 0x68:
        return None
    i += 1
    m = b[i]
    if m == 0xda:      # str16
        n = struct.unpack_from(">H", b, i + 1)[0]; i += 3
    elif m == 0xdb:    # str32
        n = struct.unpack_from(">I", b, i + 1)[0]; i += 5
    elif m == 0xc5:    # bin16
        n = struct.unpack_from(">H", b, i + 1)[0]; i += 3
    elif m == 0xc6:    # bin32
        n = struct.unpack_from(">I", b, i + 1)[0]; i += 5
    else:
        return None
    return i + n


def list_total(b):
    """If `b` opens a preset-*list* stream, return its total reassembled length, else None.

    Same envelope as a preset read — `83 66 <counter> 67 <n> 68 <value>` — but key 104 holds the
    array of per-slot entries, so `stream_total`'s str/bin markers never match. An array header
    counts elements rather than bytes, so the total comes from the 8-byte stream header instead:
    u32 LE at offset 4, plus the header. Same rule as `fretwire_data::stream::declared_stream_len`,
    which is what the reader in fretwire-core reassembles a listing by.
    """
    if len(b) < 16 or b[8] != 0x83 or b[9] != 0x66:
        return None
    i = 10
    for key in (0x67, 0x68):
        m = b[i]
        i += 1 if m < 0x80 else {0xcc: 2, 0xcd: 3, 0xce: 5}.get(m, 0)
        if i <= 10 or i + 1 > len(b) or b[i] != key:
            return None
        i += 1
    # 104 must hold an array: fixarray, array16 or array32.
    if not (0x90 <= b[i] <= 0x9f or b[i] in (0xdc, 0xdd)):
        return None
    total = struct.unpack_from("<I", b, 4)[0] + STREAM_PREFIX
    return total if STREAM_PREFIX < total <= (1 << 20) else None


def main():
    path = sys.argv[1]
    args = [a for a in sys.argv[2:] if not a.startswith("--")]
    prefix = args[0] if args else "stream"
    minsize = 1024
    if "--min" in sys.argv:
        minsize = int(sys.argv[sys.argv.index("--min") + 1])
    # Listings come off the primary channel in HX Edit's own captures and off the edit channel in
    # ours (our handshake doesn't leave primary browse-ready), so accept either.
    listing = "--list" in sys.argv
    total_of = list_total if listing else stream_total
    srcs = (EDIT_DEV, PRI_DEV) if listing else (EDIT_DEV,)

    acc, want, found = None, 0, 0
    for pkt in walk(open(path, "rb").read()):
        u = usb(pkt)
        if not u:
            continue
        ep, tr, data = u
        if tr != 3 or not data:
            continue
        f = decode_frame(data)
        if not f or f["src"] not in srcs or f["cmd"] != 0x04 or not f["body"]:
            continue
        b = f["body"]
        total = total_of(b)
        if total is not None:
            acc, want = bytearray(b), total     # a new stream starts here
        elif acc is not None:
            acc += b
        else:
            continue

        if len(acc) >= want:
            blob = bytes(acc[:want])
            if len(blob) >= minsize:
                out = f"{prefix}{found}.msgpack.bin"
                open(out, "wb").write(blob)
                print(f"wrote {out}  {len(blob)} bytes")
                found += 1
            acc, want = None, 0
    if not found:
        print(f"no streams >= {minsize} bytes found", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
