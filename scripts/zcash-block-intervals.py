#!/usr/bin/env python3
"""Measures Zcash block intervals through a lightwalletd gRPC-web endpoint.

Read-only. It asks for the latest block, then for the compact blocks of the last N
intervals (GetBlockRange, transparent pool only so that the answers stay small), and
prints the distribution of the time between consecutive blocks and of k consecutive
intervals (k = 2 and 3: what k_Z confirmations take).

    python3 -I scripts/zcash-block-intervals.py [URL] [N] [--json]

URL defaults to https://zcash-mainnet.chainsafe.dev, N to 3000. The timing tests in
crates/zecret-node/src/testnet_config.rs and mainnet_config.rs quote its output.
Block times are the miners' header timestamps, so single intervals can be negative or
skewed; the percentiles of 3000 intervals are what the configs use.
"""

import json
import os
import ssl
import statistics
import struct
import sys
import urllib.request

SERVICE = "cash.z.wallet.sdk.rpc.CompactTxStreamer"


def varint(b: bytes, i: int):
    shift = 0
    v = 0
    while True:
        c = b[i]
        i += 1
        v |= (c & 0x7F) << shift
        if not c & 0x80:
            return v, i
        shift += 7


def enc_varint(v: int) -> bytes:
    out = bytearray()
    while True:
        c = v & 0x7F
        v >>= 7
        if v:
            out.append(c | 0x80)
        else:
            out.append(c)
            return bytes(out)


def fields(b: bytes):
    """Yields (field number, wire type, value) of one protobuf message."""
    i = 0
    while i < len(b):
        key, i = varint(b, i)
        f, wt = key >> 3, key & 7
        if wt == 0:
            v, i = varint(b, i)
        elif wt == 2:
            n, i = varint(b, i)
            v = b[i : i + n]
            i += n
        elif wt == 5:
            v = struct.unpack("<I", b[i : i + 4])[0]
            i += 4
        elif wt == 1:
            v = struct.unpack("<Q", b[i : i + 8])[0]
            i += 8
        else:
            raise ValueError(f"wire type {wt}")
        yield f, wt, v


def call(url: str, method: str, msg: bytes):
    """One gRPC-web call; returns the data frames."""
    body = b"\x00" + struct.pack(">I", len(msg)) + msg
    req = urllib.request.Request(
        f"{url.rstrip('/')}/{SERVICE}/{method}",
        data=body,
        headers={"content-type": "application/grpc-web+proto", "x-grpc-web": "1"},
        method="POST",
    )
    ctx = ssl.create_default_context(cafile=os.environ.get("SSL_CERT_FILE") or None)
    with urllib.request.urlopen(req, timeout=300, context=ctx) as r:
        data = r.read()
    frames, i, trailer = [], 0, b""
    while i < len(data):
        flag = data[i]
        n = struct.unpack(">I", data[i + 1 : i + 5])[0]
        payload = data[i + 5 : i + 5 + n]
        i += 5 + n
        if flag & 0x80:
            trailer = payload
        else:
            frames.append(payload)
    if b"grpc-status:0" not in trailer.replace(b" ", b""):
        raise RuntimeError(f"{method}: {trailer.decode(errors='replace')}")
    return frames


def block_id(height: int) -> bytes:
    return b"\x08" + enc_varint(height)


def pct(xs, p):
    xs = sorted(xs)
    k = min(len(xs) - 1, max(0, int(round(p / 100 * (len(xs) - 1)))))
    return xs[k]


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    url = args[0] if args else "https://zcash-mainnet.chainsafe.dev"
    n = int(args[1]) if len(args) > 1 else 3000
    tip = {f: v for f, _, v in fields(call(url, "GetLatestBlock", b"")[0])}[1]
    start = tip - n
    times = {}
    step = 500
    for lo in range(start, tip + 1, step):
        hi = min(tip, lo + step - 1)
        # BlockRange{start, end, poolTypes=[TRANSPARENT(1)]}
        msg = b"\x0a" + enc_varint(len(block_id(lo))) + block_id(lo)
        msg += b"\x12" + enc_varint(len(block_id(hi))) + block_id(hi)
        msg += b"\x18\x01"
        for fr in call(url, "GetBlockRange", msg):
            h = t = None
            for f, _, v in fields(fr):
                if f == 2:
                    h = v
                elif f == 5:
                    t = v
            times[h] = t
    hs = sorted(times)
    iv = [times[b] - times[a] for a, b in zip(hs, hs[1:]) if b == a + 1]
    k2 = [times[hs[i + 2]] - times[hs[i]] for i in range(len(hs) - 2)]
    k3 = [times[hs[i + 3]] - times[hs[i]] for i in range(len(hs) - 3)]
    span = times[hs[-1]] - times[hs[0]]

    def dist(xs):
        return {
            "mean": round(statistics.mean(xs), 1),
            "median": statistics.median(xs),
            "p99": pct(xs, 99),
            "p99_9": pct(xs, 99.9),
            "max": max(xs),
            "min": min(xs),
        }

    out = {
        "url": url,
        "first_block": hs[0],
        "last_block": hs[-1],
        "intervals": len(iv),
        "span_secs": span,
        "one_block": dist(iv),
        "two_blocks": dist(k2),
        "three_blocks": dist(k3),
    }
    if "--json" in sys.argv:
        print(json.dumps(out, indent=2))
    else:
        print(f"{url}: blocks {hs[0]}..{hs[-1]}, {len(iv)} intervals, {span} s")
        for k in ("one_block", "two_blocks", "three_blocks"):
            d = out[k]
            print(f"  {k:12s} mean {d['mean']:>7} s  median {d['median']:>5} s  p99 {d['p99']:>5} s  p99.9 {d['p99_9']:>5} s  max {d['max']:>5} s  min {d['min']:>4} s")


if __name__ == "__main__":
    main()
