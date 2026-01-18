#!/usr/bin/env bash
set -euo pipefail

cargo build --release -p blackpepper
cp target/release/pepper ~/.local/bin/

echo "Setup done."
