#!/usr/bin/env bash
set -euo pipefail
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if [[ -f "$script_dir/whispertype" ]]; then
  binary="$script_dir/whispertype"
else
  project_root=$(cd -- "$script_dir/.." && pwd)
  cd "$project_root"
  cargo build --release --locked
  binary="$project_root/target/release/whispertype"
fi
install -Dm755 "$binary" "$HOME/.local/bin/whispertype"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}/whispertype"
mkdir -p "$data_dir"
for notice in LICENSE THIRD_PARTY_NOTICES.txt README.md; do
  if [[ -f "$script_dir/$notice" ]]; then
    install -m644 "$script_dir/$notice" "$data_dir/$notice"
  fi
done
"$HOME/.local/bin/whispertype" init
printf 'Whispertype installed. Add your key with: whispertype set-key\n'
