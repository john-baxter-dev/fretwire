#!/usr/bin/env python3
"""Decode HX edit/status MessagePack envelopes from a USBPcap capture (no external deps).

Usage:
  tools/decode-edits.py <cap.pcapng>                 # host->device EDIT command envelopes {102,100,101}
  tools/decode-edits.py <cap.pcapng> IN              # device->host envelopes (status pushes, replies)
  tools/decode-edits.py <cap.pcapng> BOTH 43 39      # both directions, only ops 43/39 (key 100)

Companion to tools/pcap-frames.py (which prints raw frames). This one strips the 8-byte TLV header
and decodes the msgpack so you can read ops/targets directly. Used to decode move(43)/add(39)/
swap(40) and the status-channel state-push {105:type,106:payload}. See docs/protocol.md.
"""
import struct, sys

CHAN = {0x1001:"PRI",0x03ef:"dev.PRI",0x1080:"EDIT",0x03ed:"dev.EDIT",0x1002:"STA",0x03f0:"dev.STA"}

def decode_frame(d):
    if len(d) < 16: return None
    ln = d[0] | (d[1]<<8); magic = d[3]
    if magic not in (0x18,0x28): return None
    return dict(src=d[4]|(d[5]<<8), dst=d[6]|(d[7]<<8), seq=d[9], cmd=d[11],
                arg=d[12]|(d[13]<<16==0 and d[13]<<8 or d[13]<<8)|(d[14]<<16)|(d[15]<<24),
                body=bytes(d[16:16+max(0,ln-8)]))

def walk(buf):
    off=0; n=len(buf)
    while off+12<=n:
        bt,bl=struct.unpack_from("<II",buf,off)
        if bl<12 or off+bl>n: break
        if bt==6:
            cap=struct.unpack_from("<I",buf,off+20)[0]
            yield buf[off+28:off+28+cap]
        off+=bl

def usb(pkt):
    if len(pkt)<27: return None
    hl=struct.unpack_from("<H",pkt,0)[0]
    if hl<27 or hl>len(pkt): return None
    ep=pkt[21]; tr=pkt[22]
    return ep,tr,("IN" if ep&0x80 else "OUT"),pkt[hl:]

class MP:
    def __init__(self,b): self.b=b; self.i=0
    def u(self,n):
        v=int.from_bytes(self.b[self.i:self.i+n],"big"); self.i+=n; return v
    def read(self):
        b=self.b[self.i]; self.i+=1
        if b<0x80: return b
        if b>=0xe0: return b-256
        if 0x80<=b<=0x8f: return {self.read():self.read() for _ in range(b&0xf)}
        if 0x90<=b<=0x9f: return [self.read() for _ in range(b&0xf)]
        if 0xa0<=b<=0xbf:
            n=b&0x1f; s=self.b[self.i:self.i+n]; self.i+=n; return s.decode("latin1")
        if b==0xc0: return None
        if b==0xc2: return False
        if b==0xc3: return True
        if b==0xca: v=struct.unpack(">f",self.b[self.i:self.i+4])[0]; self.i+=4; return round(v,4)
        if b==0xcb: v=struct.unpack(">d",self.b[self.i:self.i+8])[0]; self.i+=8; return v
        if b==0xcc: return self.u(1)
        if b==0xcd: return self.u(2)
        if b==0xce: return self.u(4)
        if b==0xcf: return self.u(8)
        if b==0xd0: v=self.u(1); return v-256 if v>=128 else v
        if b==0xd1: v=self.u(2); return v-65536 if v>=32768 else v
        if b==0xd2: v=self.u(4); return v-(1<<32) if v>=(1<<31) else v
        if b in (0xc4,0xd9): n=self.u(1); s=self.b[self.i:self.i+n]; self.i+=n; return s.decode("latin1")
        if b in (0xc5,0xda): n=self.u(2); s=self.b[self.i:self.i+n]; self.i+=n; return "<%dB>"%n
        if b==0xdc: n=self.u(2); return [self.read() for _ in range(n)]
        if b==0xde: n=self.u(2); return {self.read():self.read() for _ in range(n)}
        return "?%02x"%b

def envelope(body):
    # A true command frame starts with the 8-byte TLV header `01 00 0X 00 <len u32>` (0X = 06
    # PARAM/edit or 02 SESSION/browse). Continuation chunks of a streamed blob have no header — skip.
    if len(body) < 9 or body[0] != 0x01 or body[1] != 0x00 or body[3] != 0x00 or body[2] not in (0x06, 0x02):
        return None
    try:
        return MP(body[8:]).read()
    except Exception:
        return None

def any_envelope(body):
    # Decode an envelope regardless of TLV header presence: try header-stripped, then offset 8, then
    # a scan for a small map. For incoming device frames the header/wrapping varies.
    e = envelope(body)
    if isinstance(e, dict): return e
    for start in (8,) + tuple(range(0, 12)):
        if start < len(body) and 0x81 <= body[start] <= 0x8f:
            try:
                v = MP(body[start:]).read()
                if isinstance(v, dict): return v
            except Exception:
                continue
    return None

def main():
    path=sys.argv[1]
    direction = "OUT"
    rest = sys.argv[2:]
    if rest and rest[0] in ("IN","OUT","BOTH"):
        direction = rest[0]; rest = rest[1:]
    want_ops=set(int(x) for x in rest) if rest else None
    for pkt in walk(open(path,"rb").read()):
        u=usb(pkt)
        if not u: continue
        ep,tr,dr,data=u
        if tr!=3 or not data: continue
        if direction!="BOTH" and dr!=direction: continue
        f=decode_frame(data)
        if not f or f["cmd"]==0x10 or not f["body"]: continue
        chan = CHAN.get(f["src"]) or CHAN.get(f["dst"]) or f"{f['src']:#06x}"
        env = any_envelope(f["body"]) if direction!="OUT" else envelope(f["body"])
        if not isinstance(env,dict): continue
        op=env.get(100)
        if want_ops and op not in want_ops: continue
        print(f'{dr} {chan:8} cmd={f["cmd"]:#04x} op={op} env={env}')

if __name__=="__main__": main()
