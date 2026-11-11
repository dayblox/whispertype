#!/usr/bin/env bash
set -euo pipefail
: "${LLVM_MINGW:?Set LLVM_MINGW to the extracted LLVM-MinGW toolchain directory}"
export PATH="$LLVM_MINGW/bin:$PATH"
export CC_x86_64_pc_windows_gnullvm=x86_64-w64-mingw32-clang
export AR_x86_64_pc_windows_gnullvm=llvm-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER=x86_64-w64-mingw32-clang
rustup target add x86_64-pc-windows-gnullvm
cargo rustc --release --locked --target x86_64-pc-windows-gnullvm -- -C target-feature=+crt-static -C link-arg=-static
printf 'Windows executable: target/x86_64-pc-windows-gnullvm/release/whispertype.exe\n'
