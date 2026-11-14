#!/usr/bin/env bash
# Renders the launch film from the sources in video/src and one recorded swap.
#
#   video/render.sh <raw dir> [work dir]
#
# <raw dir>  the output folder of video/record.mjs (raw.mkv, marks.json, facts.json, boxes.json)
# [work dir] where the footage frames and film frames go (large; default: a temp folder)
#
# Writes testnet/hosted/demo/cloakroom-intents-launch.mp4 and -poster.jpg.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$HERE")"
RAW="$1"
WORK="${2:-$(mktemp -d)}"
OUT="$ROOT/testnet/hosted/demo"
WORKERS="${WORKERS:-3}"
export PLAYWRIGHT_BROWSERS_PATH="${PLAYWRIGHT_BROWSERS_PATH:-/opt/pw-browsers}"
mkdir -p "$WORK/footage" "$WORK/frames" "$OUT"

python3 "$HERE/cut_footage.py" "$RAW" "$WORK/footage"
rm -f "$WORK/frames"/*.jpg
node "$HERE/render.mjs" frames "$WORK/frames" 0 "" "$WORKERS"

ffmpeg -v error -y -framerate 30 -start_number 0 -i "$WORK/frames/f%05d.jpg" \
  -f lavfi -i anullsrc=channel_layout=stereo:sample_rate=48000 \
  -map 0:v -map 1:a -shortest \
  -c:v libx264 -preset slow -crf 19 -tune animation -pix_fmt yuv420p -profile:v high -level 4.1 \
  -x264-params keyint=60 -c:a aac -b:a 32k -movflags +faststart \
  "$OUT/cloakroom-intents-launch.mp4"

# The poster: a frame of the product scene (the finished swap) by default.
POSTER_T="${POSTER_T:-62.7}"
ffmpeg -v error -y -ss "$POSTER_T" -i "$OUT/cloakroom-intents-launch.mp4" -frames:v 1 -q:v 2 "$OUT/cloakroom-intents-launch-poster.jpg"
python3 "$HERE/make_facts.py" "$RAW"
ffprobe -v error -show_entries format=duration,size:stream=codec_name,width,height,r_frame_rate,pix_fmt -of compact "$OUT/cloakroom-intents-launch.mp4"
