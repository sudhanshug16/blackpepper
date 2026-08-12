#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
OUTPUT_DIR=${1:-"$ROOT/docs/assets"}
FONT_DIR="$ROOT/docs/assets/fonts"
REGULAR_FONT="$FONT_DIR/JetBrainsMono-Regular.ttf"
MEDIUM_FONT="$FONT_DIR/JetBrainsMono-Medium.ttf"
BOLD_FONT="$FONT_DIR/JetBrainsMono-Bold.ttf"

if ! command -v magick >/dev/null 2>&1; then
  printf '%s\n' "ImageMagick 7 is required to generate brand assets." >&2
  exit 1
fi

for font in "$REGULAR_FONT" "$MEDIUM_FONT" "$BOLD_FONT"; do
  if [ ! -f "$font" ]; then
    printf 'Missing bundled font: %s\n' "$font" >&2
    exit 1
  fi
done

mkdir -p "$OUTPUT_DIR"

png_options=(
  -strip
  -define 'png:exclude-chunks=date,time'
  -define 'png:compression-level=9'
  -define 'png:compression-strategy=1'
)

# The favicon is the supplied grinder plate: four holes on a peppercorn field.
magick -size 16x16 xc:'#e4834f' \
  -fill '#1c1d1f' \
  -draw 'rectangle 4,4 6,6 rectangle 9,4 11,6 rectangle 4,9 6,11 rectangle 9,9 11,11' \
  "${png_options[@]}" "PNG32:$OUTPUT_DIR/favicon-16.png"

magick -size 32x32 xc:'#e4834f' \
  -fill '#1c1d1f' \
  -draw 'circle 10.5,10.5 13.5,10.5 circle 20.5,10.5 23.5,10.5 circle 10.5,20.5 13.5,20.5 circle 20.5,20.5 23.5,20.5' \
  "${png_options[@]}" "PNG32:$OUTPUT_DIR/favicon-32.png"

magick -size 48x48 xc:'#e4834f' \
  -fill '#1c1d1f' \
  -draw 'circle 16,16 20.5,16 circle 32,16 36.5,16 circle 16,32 20.5,32 circle 32,32 36.5,32' \
  "${png_options[@]}" "PNG32:$OUTPUT_DIR/favicon-48.png"

rounded_icon() {
  size=$1
  radius=$2
  hole_start=$3
  hole_size=$4
  hole_gap=$5
  output=$6
  end=$((size - 1))
  hole_end=$((hole_start + hole_size - 1))
  second_start=$((hole_start + hole_size + hole_gap))
  second_end=$((second_start + hole_size - 1))

  magick -size "${size}x${size}" xc:none \
    -fill '#e4834f' -draw "roundrectangle 0,0 $end,$end $radius,$radius" \
    -fill '#1c1d1f' \
    -draw "circle $(((hole_start + hole_end) / 2)),$(((hole_start + hole_end) / 2)) $hole_end,$(((hole_start + hole_end) / 2)) \
circle $(((second_start + second_end) / 2)),$(((hole_start + hole_end) / 2)) $second_end,$(((hole_start + hole_end) / 2)) \
circle $(((hole_start + hole_end) / 2)),$(((second_start + second_end) / 2)) $hole_end,$(((second_start + second_end) / 2)) \
circle $(((second_start + second_end) / 2)),$(((second_start + second_end) / 2)) $second_end,$(((second_start + second_end) / 2))" \
    "${png_options[@]}" "PNG32:$OUTPUT_DIR/$output"
}

rounded_icon 180 40 50 30 20 apple-touch-icon.png
rounded_icon 192 43 54 32 21 app-icon-192.png
rounded_icon 512 114 143 85 57 app-icon-512.png

# Social card mirrors the supplied 1280x640 crop: flat canvas, wordmark,
# two-line product statement, and the four-row terminal mark.
magick -size 1280x640 xc:'#141517' \
  -font "$MEDIUM_FONT" -pointsize 52 -fill '#e6e4e1' -annotate +128+278 'black' \
  -fill '#e4834f' -annotate +284+278 'pepper' \
  -font "$REGULAR_FONT" -pointsize 22 -fill '#8b8a87' \
  -annotate +128+348 'one UI for local folders and Linux SSH hosts.' \
  -annotate +128+385 'zellij keeps the work alive where it runs.' \
  -font "$BOLD_FONT" -pointsize 38 -fill '#e4834f' -interline-spacing -2 \
  -annotate +900+215 $'█\n█▀▄  █▀▄\n█▄▀  █▄▀\n     █' \
  "${png_options[@]}" "PNG32:$OUTPUT_DIR/social-card.png"

chmod 0644 \
  "$OUTPUT_DIR/favicon-16.png" "$OUTPUT_DIR/favicon-32.png" \
  "$OUTPUT_DIR/favicon-48.png" "$OUTPUT_DIR/apple-touch-icon.png" \
  "$OUTPUT_DIR/app-icon-192.png" "$OUTPUT_DIR/app-icon-512.png" \
  "$OUTPUT_DIR/social-card.png"

(
  cd "$OUTPUT_DIR"
  sha256sum \
    favicon-16.png favicon-32.png favicon-48.png \
    apple-touch-icon.png app-icon-192.png app-icon-512.png social-card.png \
    > brand-sha256s.txt
)

printf 'Generated Blackpepper brand assets in %s\n' "$OUTPUT_DIR"
