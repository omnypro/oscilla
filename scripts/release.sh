#!/bin/bash
# Builds the release artifact for a version and rewrites the manifest to use it.
#
#   scripts/release.sh VERSION [--dry-run]
#
# VERSION must be the newest heading in CHANGES.md. The script builds the
# XCFramework, zips it, computes the SwiftPM checksum and changes the
# binaryTarget in Package.swift from the local path to the release URL. It does
# not commit, tag or push. With --dry-run it writes the changed manifest to
# build/Package.release.swift and leaves Package.swift alone.
set -euo pipefail
cd "$(dirname "$0")/.."

version=${1:?usage: scripts/release.sh VERSION [--dry-run]}
dry_run=false
[ "${2:-}" = "--dry-run" ] && dry_run=true

newest=$(sed -n 's/^## \([0-9][0-9.]*\)$/\1/p' CHANGES.md | head -n 1)
if [ "$version" != "$newest" ]; then
    echo "CHANGES.md starts with $newest, not $version." >&2
    exit 1
fi

scripts/build-xcframework.sh

zip="build/OscillaRust-$version.xcframework.zip"
rm -f "$zip"
ditto -c -k --sequesterRsrc --keepParent build/OscillaRust.xcframework "$zip"
checksum=$(swift package compute-checksum "$zip")

repository=${GITHUB_REPOSITORY:-omnypro/oscilla}
url="https://github.com/$repository/releases/download/v$version/OscillaRust-$version.xcframework.zip"

manifest=Package.swift
$dry_run && manifest=build/Package.release.swift
python3 -I - "$manifest" "$url" "$checksum" <<'PY'
import re, sys
out, url, checksum = sys.argv[1:4]
text = open("Package.swift").read()
old = re.compile(r'\.binaryTarget\(\s*name: "OscillaRust",\s*path: "build/OscillaRust\.xcframework"\s*\)')
if not old.search(text):
    sys.exit("The binaryTarget for OscillaRust was not found in Package.swift.")
new = '.binaryTarget(\n            name: "OscillaRust",\n            url: "%s",\n            checksum: "%s"\n        )' % (url, checksum)
open(out, "w").write(old.sub(new, text, count=1))
PY

echo "zip      $zip"
echo "checksum $checksum"
echo "url      $url"
