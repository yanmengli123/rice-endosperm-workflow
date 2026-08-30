"""Strip `+reference-types` from a wasm's target_features custom section.

This prevents wasm-bindgen-cli <0.2.100 from auto-enabling externref-xform,
which produces an externref table that webkit2gtk 2.52.3 ARM64 cannot expose
in instance.exports (a real engine bug, breaks wisp-tauri's webview).

Usage: python3 strip_reference_types.py <wasm_path>
Writes <wasm_path>.no_reftypes as the patched wasm.
"""
import sys, shutil

def leb(v):
    out = bytearray()
    while True:
        b = v & 0x7f
        v >>= 7
        if v:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)

def read_uleb(data, off):
    n = 0; sh = 0
    while True:
        b = data[off]; off += 1
        n |= (b & 0x7f) << sh
        if not (b & 0x80): return n, off
        sh += 7

def main(path):
    with open(path, 'rb') as f:
        data = bytearray(f.read())
    assert data[:4] == b'\x00asm', "not a wasm"
    i = 8  # skip magic+version
    new_data = bytearray(data[:8])
    while i < len(data):
        sid = data[i]; i += 1
        sz_start = i
        size, i = read_uleb(data, i)
        section_body_start = i
        section_body_end = i + size
        if sid == 0:  # custom
            # parse name
            name_len, off = read_uleb(data, section_body_start)
            name = data[section_body_start + (off - section_body_start): section_body_start + (off - section_body_start) + name_len]
            # simpler: just decode name properly
            p = section_body_start
            nl, p = read_uleb(data, p)
            name = bytes(data[p:p+nl]); p += nl
            if name == b'target_features':
                # parse entries
                cnt, p2 = read_uleb(data, p)
                entries_start = p2
                kept = []
                pp = p2
                for _ in range(cnt):
                    prefix = chr(data[pp]); pp += 1
                    fl, pp = read_uleb(data, pp)
                    fname = bytes(data[pp:pp+fl]); pp += fl
                    if prefix == '+' and fname == b'reference-types':
                        print(f"  dropping +reference-types", file=sys.stderr)
                        continue
                    kept.append((prefix, fname))
                # rebuild section body: name + leb(cnt) + entries
                body = bytearray()
                body += leb(nl) + name
                body += leb(len(kept))
                for prefix, fname in kept:
                    body += bytes([ord(prefix)])
                    body += leb(len(fname)) + fname
                # rebuild section: sid + leb(len(body)) + body
                new_data += bytes([sid]) + leb(len(body)) + bytes(body)
                i = section_body_end
                continue
        # default: copy section as-is
        # bytes from sid..section_body_end
        section_bytes = bytes(data[sz_start - 1: section_body_end])
        # but we already consumed sid and the size; reconstruct:
        # actually simpler: copy from the original data[start_i-1 : section_body_end]
        # where start_i-1 was the position of sid
        # We've been appending into new_data; here just append the raw section bytes.
        # The raw bytes start at (sz_start - 1) — that's where sid was.
        new_data += data[sz_start - 1: section_body_end]
        i = section_body_end
    out = path + '.no_reftypes'
    with open(out, 'wb') as f:
        f.write(new_data)
    print(f"wrote {out}", file=sys.stderr)
    return out

if __name__ == '__main__':
    main(sys.argv[1])
