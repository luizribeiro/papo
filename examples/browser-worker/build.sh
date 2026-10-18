#!/usr/bin/env bash
set -euo pipefail

example_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd "$example_dir/../.." && pwd)
output_dir=$example_dir/dist

(cd "$repo_root" && cargo build -p papo-browser-worker --target wasm32-unknown-unknown --release)
wasm-bindgen \
  "$repo_root/target/wasm32-unknown-unknown/release/papo_browser_worker.wasm" \
  --target web --out-dir "$output_dir/pkg"
cp "$example_dir/index.html" "$example_dir/worker.js" "$output_dir/"
