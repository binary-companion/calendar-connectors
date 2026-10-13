#!/usr/bin/env bash
# Select the newest Xcode installed under /Applications and print toolchain versions.
#
# Used by CI so the build gets the newest SDK the runner image offers (the app uses macOS 26
# APIs behind #available checks, so Xcode 26 or newer is required).
#
# Environment: none required. Uses sudo, so it is meant for CI runners.
set -euo pipefail

newest=""
# Version-sort so Xcode_26.10.app sorts after Xcode_26.2.app; plain "Xcode.app" is a fallback.
while IFS= read -r app; do
  newest="$app"
done < <(find /Applications -maxdepth 1 -name 'Xcode*.app' 2>/dev/null | sort -V)

if [ -z "$newest" ]; then
  echo "error: no /Applications/Xcode*.app found on this runner" >&2
  exit 1
fi

echo "Selecting $newest"
sudo xcode-select -s "$newest/Contents/Developer"
xcodebuild -version
swift --version
