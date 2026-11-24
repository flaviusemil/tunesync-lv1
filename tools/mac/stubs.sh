#!/bin/bash
# stubs.sh <arch> <sdk dir> <imports file> - link stubs (.tbd, text-based dylib stubs) for
# the system libraries TuneSync links against, naming only the symbols in <imports> (one
# Mach-O symbol per line, from llvm-nm -u; /dev/null for empty stubs). Used by cc.sh.
#
# Each symbol goes to its library by name:
#   _objc_* _sel_* _class_* _object_*   libobjc
#   _CF* _kCF*     CoreFoundation        _CG* _kCG*      CoreGraphics
#   _CT* _kCT*     CoreText              _MIDI* _kMIDI*  CoreMIDI
#   _NSPasteboardTypeString _NSAppearanceNameDarkAqua    AppKit
#   _iconv* _libiconv*  libiconv (its own dylib; the libc crate asks for -liconv on Apple)
#   anything else  libSystem (libc, pthreads, libdispatch, dns_sd, Mach, dyld, ...: all
#                  re-exported by /usr/lib/libSystem.B.dylib, as in Apple's own SDK)
# Any other _NS* or _OBJC_* symbol (Foundation, ObjC class references) stops the build:
# add it here with its real library.
set -euo pipefail
arch=$1; sdk=$2; imports=$3
fw=/System/Library/Frameworks
mkdir -p "$sdk/usr/lib"
for f in CoreFoundation CoreGraphics CoreText CoreMIDI AppKit; do mkdir -p "$sdk$fw/$f.framework"; done

bad=$(grep -E '^(_NS|_OBJC_)' "$imports" | grep -vxE '_NSPasteboardTypeString|_NSAppearanceNameDarkAqua' || true)
if [ -n "$bad" ]; then
    echo "stubs.sh: no library known for: $(echo $bad)" >&2
    echo "stubs.sh: add them to tools/mac/stubs.sh with the library that exports them" >&2
    exit 1
fi

# stub <file> <install name> <extra symbols> <grep -E pattern | -v pattern>
stub() {
    local file=$1 name=$2 extra=$3 syms
    shift 3
    syms=$( (grep -E "$@" "$imports" || true; for x in $extra; do echo "$x"; done) | sed "s/.*/'&'/" | paste -sd, - | sed 's/,/, /g')
    {
        echo '--- !tapi-tbd'
        echo 'tbd-version:     4'
        echo "targets:         [ $arch-macos ]"
        echo "install-name:    '$name'"
        if [ -n "$syms" ]; then
            echo 'exports:'
            echo "  - targets:         [ $arch-macos ]"
            echo "    symbols:         [ $syms ]"
        fi
        echo '...'
    } > "$file"
}

objc='^_(objc|sel|class|object)_'
cf='^_k?CF'; cg='^_k?CG'; ct='^_k?CT'; midi='^_k?MIDI'; appkit='^_NS'; iconv='^_(lib)?iconv'
stub "$sdk/usr/lib/libobjc.tbd" /usr/lib/libobjc.A.dylib "" "$objc"
stub "$sdk$fw/CoreFoundation.framework/CoreFoundation.tbd" "$fw/CoreFoundation.framework/Versions/A/CoreFoundation" "" "$cf"
stub "$sdk$fw/CoreGraphics.framework/CoreGraphics.tbd" "$fw/CoreGraphics.framework/Versions/A/CoreGraphics" "" "$cg"
stub "$sdk$fw/CoreText.framework/CoreText.tbd" "$fw/CoreText.framework/Versions/A/CoreText" "" "$ct"
stub "$sdk$fw/CoreMIDI.framework/CoreMIDI.tbd" "$fw/CoreMIDI.framework/Versions/A/CoreMIDI" "" "$midi"
stub "$sdk$fw/AppKit.framework/AppKit.tbd" "$fw/AppKit.framework/Versions/C/AppKit" "" "$appkit"
stub "$sdk/usr/lib/libiconv.tbd" /usr/lib/libiconv.2.dylib "" "$iconv"
# dyld_stub_binder: the linker's own reference for lazy binding (x86_64), not an import nm lists
stub "$sdk/usr/lib/libSystem.tbd" /usr/lib/libSystem.B.dylib dyld_stub_binder -v "$objc|$cf|$cg|$ct|$midi|$appkit|$iconv"
ln -sf libSystem.tbd "$sdk/usr/lib/libc.tbd"
ln -sf libSystem.tbd "$sdk/usr/lib/libm.tbd"
