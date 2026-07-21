#!/usr/bin/env python3
"""Extract HX control frames from a USBPcap pcapng capture (read-only).

Walks pcapng blocks -> Enhanced Packet Blocks -> USBPcap packet header -> payload,
then decodes the payload as our 16-byte HX frame header + body and prints a summary.
"""
import struct, sys

CHAN = {0x1001:"host.PRIMARY",0x03ef:"dev.PRIMARY",0x1080:"host.EDIT",0x03ed:"dev.EDIT",
        0x1002:"host.STATUS",0x03f0:"dev.STATUS"}

def chan(x): return CHAN.get(x, f"{x:#06x}")

def decode_frame(d):
    if len(d) < 16: return None
    ln  = d[0] | (d[1]<<8)
    magic = d[3]
    if magic not in (0x18,0x28): return None
    src = d[4] | (d[5]<<8)
    dst = d[6] | (d[7]<<8)
    seq = d[9]; cmd = d[11]
    arg = d[12] | (d[13]<<8) | (d[14]<<16) | (d[15]<<24)
    body_len = max(0, ln - 8)
    body = d[16:16+body_len]
    return dict(src=src,dst=dst,seq=seq,cmd=cmd,arg=arg,body=bytes(body))

def walk_pcapng(buf):
    """Yield (epb_payload_bytes,) for each Enhanced Packet Block."""
    off = 0; n = len(buf)
    while off + 12 <= n:
        btype, blen = struct.unpack_from("<II", buf, off)
        if blen < 12 or off+blen > n: break
        if btype == 0x00000006:  # Enhanced Packet Block
            # type(4),blen(4),iface(4),ts_hi(4),ts_lo(4),caplen(4),origlen(4),data...
            caplen = struct.unpack_from("<I", buf, off+20)[0]
            data = buf[off+28:off+28+caplen]   # data begins after origlen (off+24)
            yield data
        off += blen
    return

def usbpcap_payload(pkt):
    """Strip the USBPcap header; return (endpoint, transfer, direction, data)."""
    if len(pkt) < 27: return None
    hdrlen = struct.unpack_from("<H", pkt, 0)[0]
    if hdrlen < 27 or hdrlen > len(pkt): return None
    endpoint = pkt[21]      # bit7 = direction (1=IN)
    transfer = pkt[22]      # 0=iso,1=int,2=ctl,3=bulk
    data = pkt[hdrlen:]
    return endpoint, transfer, ("IN" if endpoint & 0x80 else "OUT"), data

def main():
    path = sys.argv[1]
    limit = int(sys.argv[2]) if len(sys.argv) > 2 else 10**9
    buf = open(path,"rb").read()
    shown = 0
    for pkt in walk_pcapng(buf):
        up = usbpcap_payload(pkt)
        if not up: continue
        ep, transfer, direction, data = up
        if transfer != 3: continue           # bulk only
        if not data: continue
        f = decode_frame(data)
        if not f: continue
        b = f["body"]
        print(f'{direction:3} {chan(f["src"]):>13}->{chan(f["dst"]):<13} '
              f'seq={f["seq"]:#04x} cmd={f["cmd"]:#04x} arg={f["arg"]:#06x} '
              f'blen={len(b)} body={b.hex()}')
        shown += 1
        if shown >= limit: break
    print(f"\n[{shown} frames]")

if __name__ == "__main__":
    main()
