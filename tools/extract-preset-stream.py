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

Usage:
  tools/extract-preset-stream.py <cap.pcapng> [out-prefix] [--min BYTES]

Self-check: run it on `captures/open_two_presets_one_after_another.pcapng` and the first stream
should be byte-identical to `captures/preset1_stream.msgpack.bin`.

Companion to pcap-frames.py (raw frames) and decode-edits.py (envelopes). Reassembly used to be
done by hand — see captures/open_two_presets.md.
"""
import struct, sys

EDIT_DEV = 0x03ed  # dev.EDIT -> host.EDIT


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


def main():
    path = sys.argv[1]
    args = [a for a in sys.argv[2:] if not a.startswith("--")]
    prefix = args[0] if args else "stream"
    minsize = 1024
    if "--min" in sys.argv:
        minsize = int(sys.argv[sys.argv.index("--min") + 1])

    acc, want, found = None, 0, 0
    for pkt in walk(open(path, "rb").read()):
        u = usb(pkt)
        if not u:
            continue
        ep, tr, data = u
        if tr != 3 or not data:
            continue
        f = decode_frame(data)
        if not f or f["src"] != EDIT_DEV or f["cmd"] != 0x04 or not f["body"]:
            continue
        b = f["body"]
        total = stream_total(b)
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
