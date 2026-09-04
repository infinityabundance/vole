#!/bin/sh
# Phase T — FFV1 operational comparison harness (master brief §57 / Phase-T).
#
# External, NON-NORMATIVE, never part of the crate: FFmpeg's FFV1 lossless
# codec runs only here, as a benchmark baseline. VOLE normative code never
# calls external codecs. The harness records a receipt (sizes, wall times,
# environment) so a comparison is never a bare claim.
#
# Usage:
#   corpus/ffv1-compare.sh <vole-bin> <in.vole> <width> <height> <frames> <workdir>
#
#   vole-bin   path to a built `vole` binary (cargo build --release)
#   in.vole    a standalone .vole stream (Gray8, width x height, `frames` frames)
#   workdir    scratch + receipt directory (created)
#
# The harness:
#   1. `vole decode` the .vole to raw Gray8 frames;
#   2. encodes the raw frames with FFmpeg FFV1 (lossless) into an .mkv;
#   3. decodes the .mkv back to raw Gray8;
#   4. byte-compares the two raw rasters (lossless roundtrip proof);
#   5. prints and saves a receipt with tool versions, sizes, and wall times.
#
# Exit codes: 0 = receipt written; 1 = ffmpeg unavailable (skipped, still
# writes a receipt noting the skip); 2 = harness misuse.
set -u

VOLE_BIN=${1:-}
IN_VOLE=${2:-}
W=${3:-}
H=${4:-}
FRAMES=${5:-}
WORK=${6:-}

if [ -z "$VOLE_BIN" ] || [ -z "$IN_VOLE" ] || [ -z "$W" ] || [ -z "$H" ] || [ -z "$FRAMES" ] || [ -z "$WORK" ]; then
    echo "usage: corpus/ffv1-compare.sh <vole-bin> <in.vole> <width> <height> <frames> <workdir>" >&2
    exit 2
fi

mkdir -p "$WORK"
RAW="$WORK/frames.raw"
MKV="$WORK/ffv1.mkv"
RAW2="$WORK/ffv1-decoded.raw"
RECEIPT="$WORK/ffv1-receipt.txt"
T0=$(date +%s)

echo "ffv1-compare: $IN_VOLE (${W}x${H} x$FRAMES)"
echo "vole bin: $VOLE_BIN" | tee "$RECEIPT"
"$VOLE_BIN" decode "$IN_VOLE" "$WORK/frames" >/dev/null || { echo "vole decode failed" >&2; exit 2; }
cat "$WORK"/frames/frame-*.gray > "$RAW"
SIZE_VOLE=$(wc -c < "$IN_VOLE")
SIZE_RAW=$(wc -c < "$RAW")
echo "vole stream: ${SIZE_VOLE} B; raw Gray8: ${SIZE_RAW} B" | tee -a "$RECEIPT"

if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "ffmpeg not found: FFV1 leg skipped (external harness requires it); receipt written" | tee -a "$RECEIPT"
    exit 1
fi

FFMPEG_VERSION=$(ffmpeg -version 2>/dev/null | head -n1)
echo "ffmpeg: $FFMPEG_VERSION" | tee -a "$RECEIPT"
S1=$(date +%s%N)
ffmpeg -y -loglevel error -f rawvideo -pix_fmt gray -s "${W}x${H}" -r 25 -i "$RAW" -c:v ffv1 -level 3 "$MKV"
S2=$(date +%s%N)
ffmpeg -y -loglevel error -i "$MKV" -f rawvideo -pix_fmt gray "$RAW2"
S3=$(date +%s%N)
SIZE_FFV1=$(wc -c < "$MKV")

if cmp -s "$RAW" "$RAW2"; then
    EXACT="yes"
else
    EXACT="no (FFV1 roundtrip diverged — baseline error)"
fi
echo "ffv1 stream: ${SIZE_FFV1} B; decode byte-identical to source raw: $EXACT" | tee -a "$RECEIPT"
echo "ffv1 encode ms: $(( (S2 - S1) / 1000000 )); ffv1 decode ms: $(( (S3 - S2) / 1000000 ))" | tee -a "$RECEIPT"
T1=$(date +%s)
echo "wall seconds: $(( T1 - T0 ))" | tee -a "$RECEIPT"
echo "receipt: $RECEIPT"
