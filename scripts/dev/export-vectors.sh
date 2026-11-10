#!/usr/bin/env bash
# Rebuilds the golden vectors under vectors/ from the Swift code (conventions section 9). The exporter reads internal
# entry points with @testable import, so it is built with -Xswiftc -enable-testing. Deterministic: run it twice and
# `git diff --exit-code vectors/` shows nothing.
# Usage: scripts/dev/export-vectors.sh [output-directory]   (default: vectors under the repository root)
set -euo pipefail
cd "$(dirname "$0")/../.."
out="${1:-vectors}"
swift build -c release --product VectorExporter -Xswiftc -enable-testing
"$(swift build -c release --product VectorExporter -Xswiftc -enable-testing --show-bin-path)/VectorExporter" "$out"
