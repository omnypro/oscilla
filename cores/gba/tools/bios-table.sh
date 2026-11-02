#!/bin/sh
# Assembles bios.s and prints the word table that BIOS_IMAGE in
# rust/src/bios.rs holds. The assembler only checks the hand-written routine; the build does
# not need it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
clang -target armv4t-none-eabi -c "$here/bios.s" -o "$tmp/bios.o"
xcrun llvm-objdump -d "$tmp/bios.o" | python3 -I -c '
import re, sys
for line in sys.stdin:
    d = re.match(r"\s+([0-9a-f]+):\s+([0-9a-f]{2}) ([0-9a-f]{2}) ([0-9a-f]{2}) ([0-9a-f]{2})\s+\.word\s+(0x[0-9a-f]+)", line)
    if d:
        print("    (0x%03X, 0x%08X), // data" % (int(d.group(1), 16), int(d.group(6), 16)))
        continue
    m = re.match(r"\s+([0-9a-f]+):\s+([0-9a-f]{8})\s+(.*)", line)
    if m and int(m.group(2), 16):
        text = re.sub(r"\s+", " ", m.group(3).split("@")[0].strip())
        print("    (0x%03X, 0x%08X), // %s" % (int(m.group(1), 16), int(m.group(2), 16), text))
'
