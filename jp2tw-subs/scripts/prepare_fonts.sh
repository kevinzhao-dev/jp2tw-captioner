#!/usr/bin/env bash
set -euo pipefail

# Prepare a reliable fonts directory for burn-in subtitles.
# - Copies existing Noto CJK fonts from common system locations into ./fonts
# - If none are found, prints guidance to install Noto CJK.

DEST_DIR=${1:-"./fonts"}
mkdir -p "$DEST_DIR"

found_any=0

copy_if_exists() {
  local pattern="$1"
  local base="$2"
  shopt -s nullglob
  for f in $pattern; do
    echo "Copying: $f -> $DEST_DIR/"
    cp -f "$f" "$DEST_DIR/"
    found_any=1
  done
  shopt -u nullglob
}

os=$(uname -s)
case "$os" in
  Darwin)
    copy_if_exists "$HOME/Library/Fonts/*Noto*CJ*K*TC*.*" "$HOME/Library/Fonts"
    copy_if_exists "/Library/Fonts/*Noto*CJ*K*TC*.*" "/Library/Fonts"
    copy_if_exists "/System/Library/Fonts/*Noto*CJ*K*TC*.*" "/System/Library/Fonts" || true
    ;;
  Linux)
    copy_if_exists "/usr/share/fonts/*Noto*CJ*K*TC*.*" "/usr/share/fonts"
    copy_if_exists "/usr/share/fonts/truetype/*Noto*CJ*K*TC*.*" "/usr/share/fonts/truetype"
    copy_if_exists "/usr/local/share/fonts/*Noto*CJ*K*TC*.*" "/usr/local/share/fonts"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    copy_if_exists "/c/Windows/Fonts/*Noto*CJ*K*TC*.*" "/c/Windows/Fonts"
    ;;
esac

if [ "$found_any" -eq 1 ]; then
  echo "Fonts prepared in $DEST_DIR"
  echo "Tip: run with --font-dir $DEST_DIR or set JP2TW_subs_FONTS_DIR=$DEST_DIR"
  exit 0
fi

cat <<EOT
No Noto CJK TC fonts found on this system.
Install them, then re-run this script. Examples:

macOS (Homebrew):
  brew install --cask font-noto-sans-cjk
  brew install --cask font-noto-serif-cjk

Linux (Debian/Ubuntu):
  sudo apt-get install fonts-noto-cjk

After installation, run:
  scripts/prepare_fonts.sh

Then use:
  ./target/debug/jp2tw-subs ... --burn-in --font-dir "$DEST_DIR" --font-name "Noto Sans CJK TC"
Or set:
  export JP2TW_subs_FONTS_DIR="$DEST_DIR"
EOT
exit 1
