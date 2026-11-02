#!/bin/bash
# Builds the Rust library for macOS, iOS and the iOS simulator and wraps the
# slices in one XCFramework that Package.swift uses as a binary target.
# macOS and the simulator are universal (arm64 and x86_64) so that generic
# destinations link.
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=15.0 IPHONEOS_DEPLOYMENT_TARGET=18.0

# The Rust objects carry these as their minimum versions, and a package that
# declares an older minimum would link them into a build that cannot run them.
# Package.swift must declare the same ones.
if ! grep -qF "platforms: [.macOS(\"$MACOSX_DEPLOYMENT_TARGET\"), .iOS(\"$IPHONEOS_DEPLOYMENT_TARGET\")]" Package.swift; then
    echo "Package.swift and this script disagree about the deployment targets." >&2
    exit 1
fi
lib=liboscilla_ffi.a
out=build/slices
rm -rf build/OscillaRust.xcframework "$out"
mkdir -p "$out"/macos "$out"/ios "$out"/ios-sim

for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios \
         aarch64-apple-ios-sim x86_64-apple-ios; do
    cargo build --locked --release -p oscilla-ffi --target "$t"
done

built() { echo "target/$1/release/$lib"; }
lipo -create "$(built aarch64-apple-darwin)" "$(built x86_64-apple-darwin)" -output "$out/macos/$lib"
cp "$(built aarch64-apple-ios)" "$out/ios/$lib"
lipo -create "$(built aarch64-apple-ios-sim)" "$(built x86_64-apple-ios)" -output "$out/ios-sim/$lib"

xcodebuild -create-xcframework \
    -library "$out/macos/$lib" -library "$out/ios/$lib" -library "$out/ios-sim/$lib" \
    -output build/OscillaRust.xcframework
