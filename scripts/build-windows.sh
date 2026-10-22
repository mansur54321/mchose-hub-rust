#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
compiler="$project_dir/.tools/rust-sysroot/bin/rustc"
toolchain="$project_dir/.tools/llvm-mingw/bin"
if [ ! -x "$compiler" ] || [ ! -x "$toolchain/x86_64-w64-mingw32-clang" ]; then
    printf '%s\n' 'Missing project-local toolchain; see docs/windows.md.' >&2
    exit 1
fi
export RUSTC="$compiler"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUSTFLAGS="-C target-feature=+crt-static"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$toolchain/x86_64-w64-mingw32-clang"
export CC_x86_64_pc_windows_gnullvm="$toolchain/x86_64-w64-mingw32-clang"
export AR_x86_64_pc_windows_gnullvm="$toolchain/llvm-ar"
export WINDRES="$toolchain/llvm-windres"
export AR="$toolchain/llvm-ar"
export PATH="$toolchain:$PATH"
if [ "${1-}" = "--test" ]; then
    shift
    exec cargo test --locked --target x86_64-pc-windows-gnullvm --release --all-targets --no-run -j 1 "$@"
fi
exec cargo build --locked --target x86_64-pc-windows-gnullvm --release --bin mchose-native -j 1 "$@"
