#!/bin/bash
# 基准音频生成脚本。
# 用 macOS `say` 合成文本真值精确已知的语音片段，再用应用自带的 ffmpeg
# 转成应用支持导入的 m4a。重新生成会覆盖 clips/ 下的音频，但真值文本不变。
#
# 前置：macOS（say 内置）；FFMPEG 指向可用的 ffmpeg（默认用应用自带的）。
# 注意：应用自带的 ffmpeg 是精简构建、不带 AIFF 解码器，
#       所以 say 统一用 --data-format=LEI16@22050 直接输出 WAVE。
# 用法：./generate.sh
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
CLIPS="$DIR/clips"
# 优先用系统完整版 ffmpeg（应用自带的是精简构建，缺 mp4/aac 封装器）
FFMPEG="${FFMPEG:-$(command -v ffmpeg || echo /Users/yoursmac/Desktop/SOLOPLAY/AI录音工作助手/app/src-tauri/resources/ffmpeg)}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ZH_FEMALE="${ZH_FEMALE:-Tingting}"                         # 林悦 / 独白
ZH_MALE="${ZH_MALE:-Eddy (中文（中国大陆）)}"                # 陈工
EN_VOICE="${EN_VOICE:-Samantha}"                           # 英文片段

command -v say >/dev/null || { echo "缺少 say"; exit 1; }
[ -x "$FFMPEG" ] || { echo "缺少 ffmpeg: $FFMPEG"; exit 1; }

gen_wave() { # $1 voice, $2 text file, $3 out.wav
  say -v "$1" -f "$2" --data-format=LEI16@22050 -o "$3"
}

to_m4a() { # $1 in.wav, $2 out.m4a
  "$FFMPEG" -y -loglevel error -i "$1" -c:a aac -b:a 96k "$2"
}

# ---- 01 短中文 / 02 中长中文：单人独白 ----
for id in 01-short 02-medium; do
  gen_wave "$ZH_FEMALE" "$CLIPS/$id.txt" "$WORK/$id.wav"
  to_m4a "$WORK/$id.wav" "$CLIPS/$id.m4a"
  echo "ok $id"
done

# ---- 05 英文 ----
if say -v "$EN_VOICE" -o "$WORK/05-probe.wav" --data-format=LEI16@22050 "probe" 2>/dev/null; then
  :
else
  EN_VOICE="Daniel"
fi
gen_wave "$EN_VOICE" "$CLIPS/05-english.txt" "$WORK/05.wav"
to_m4a "$WORK/05.wav" "$CLIPS/05-english.m4a"
echo "ok 05-english ($EN_VOICE)"

# ---- 03 双说话人：逐轮生成后拼接，轮间 0.3s 静默 ----
# 真值轮次从 03-dialogue.txt 解析（说话人 1 = 林悦 → 女声；陈工 → 男声）。
turns_file="$WORK/turns.txt"
awk -F'：' '/^[^：]+：/ { name=$1; $1=""; sub(/^：/, "", $0); print name "\t" $0 }' \
  "$CLIPS/03-dialogue.txt" > "$turns_file"

i=0
: > "$WORK/03-list.txt"
silence="$WORK/silence.wav"
"$FFMPEG" -y -loglevel error -f lavfi -i anullsrc=r=22050:cl=mono -t 0.3 "$silence"
first_turn=1
while IFS=$'\t' read -r name text; do
  [ -z "$name" ] && continue
  i=$((i + 1))
  tfile="$WORK/turn-$i.txt"
  printf '%s' "$text" > "$tfile"
  if [ "$name" = "林悦" ]; then voice="$ZH_FEMALE"; else voice="$ZH_MALE"; fi
  turn_wav="$WORK/turn-$i.wav"
  gen_wave "$voice" "$tfile" "$turn_wav"
  if [ "$first_turn" = "1" ]; then first_turn=0; else
    echo "file '$silence'" >> "$WORK/03-list.txt"
  fi
  echo "file '$turn_wav'" >> "$WORK/03-list.txt"
done < "$turns_file"

"$FFMPEG" -y -loglevel error -f concat -safe 0 -i "$WORK/03-list.txt" "$WORK/03.wav"
to_m4a "$WORK/03.wav" "$CLIPS/03-dialogue.m4a"
echo "ok 03-dialogue ($i 轮)"

# ---- 04 噪声版：02 混入粉噪声（约 15% 强度），抗噪对照 ----
"$FFMPEG" -y -loglevel error -i "$CLIPS/02-medium.m4a" \
  -f lavfi -i "anoisesrc=color=pink:amplitude=0.05" \
  -filter_complex "[0:a][1:a]amix=inputs=2:duration=first:weights='1 0.18'[a]" \
  -map "[a]" -c:a aac -b:a 96k "$CLIPS/04-noisy.m4a"
echo "ok 04-noisy"

echo "全部片段已生成到 $CLIPS"
