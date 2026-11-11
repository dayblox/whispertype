#!/usr/bin/env bash
set -euo pipefail
project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_root"
cargo build --release --locked
install -Dm755 target/release/whispertype "$HOME/.local/bin/whispertype"
"$HOME/.local/bin/whispertype" init
printf 'Whispertype installed. Add your key with: whispertype set-key\n'
