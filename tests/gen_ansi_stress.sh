#!/usr/bin/env bash
# Generates stress_ansi.txt — a raw text file with embedded ANSI SGR
# escape sequences, max coverage of colors and font attributes. `cat` the
# output file in a terminal to reproduce colors; open the herdr-flash
# popup on that pane to verify the spike reproduces them.
#
# Coverage:
#   1.  16 basic colors (fg + bg)
#   2.  8 bright colors (fg + bg)
#   3.  256-color palette (fg sweep + bg sweep)
#   4.  24-bit truecolor RGB ramp + primaries
#   5.  Attributes: bold, dim, italic, underline, blink, reverse, hidden, strikethrough
#   6.  Combined attributes (bold+underline+color, italic+reverse, etc.)
#   7.  Reset behavior (explicit reset, implicit via new style)
#   8.  State carry across newlines (no reset before \n)
#   9.  Overlapping / nested styles on one line
#  10.  Wide chars (emoji, CJK) with color
#  11.  Empty / blank lines with active color state
#  12.  Long line (for horizontal scroll + overflow `…` test) with color
#  13.  Mixed: foreground color over background color, reset fg only, reset bg only
#  14.  Default fg with colored bg, and vice versa
#  15.  Cursor-overlay candidates: colored text the cursor will land on

set -euo pipefail
out="$(dirname "$0")/stress_ansi.txt"
: > "$out"
emit() { printf '%b' "$1" >> "$out"; }
line() { emit "$1\n"; }
sec()  { emit "\n\033[1;4m### $1 ###\033[0m\n\n"; }

ESC=$'\033'

# ── 1. 16 basic colors ────────────────────────────────────────────────────────
sec "1. 16 basic colors — fg then bg"
names=(black red green yellow blue magenta cyan white)
for i in "${!names[@]}"; do
  fg=$((30 + i))
  bg=$((40 + i))
  emit "${ESC}[${fg}mfg ${names[$i]}${ESC}[0m   "
  emit "${ESC}[${bg}m bg ${names[$i]} ${ESC}[0m"
  line ""
done

# ── 2. 8 bright colors (90-97 fg, 100-107 bg) ────────────────────────────────
sec "2. bright colors — fg (90-97) then bg (100-107)"
for i in "${!names[@]}"; do
  fg=$((90 + i))
  bg=$((100 + i))
  emit "${ESC}[${fg}mbright fg ${names[$i]}${ESC}[0m   "
  emit "${ESC}[${bg}m bright bg ${names[$i]} ${ESC}[0m"
  line ""
done

# ── 3. 256-color palette ──────────────────────────────────────────────────────
sec "3. 256-color palette — fg sweep (cols of 8)"
for n in $(seq 0 255); do
  emit "${ESC}[38;5;${n}m$(printf '%3d' "$n")${ESC}[0m "
  if (((n + 1) % 8 == 0)); then line ""; fi
done

line ""
sec "3b. 256-color palette — bg sweep (cols of 8)"
for n in $(seq 0 255); do
  emit "${ESC}[48;5;${n}m $(printf '%3d' "$n") ${ESC}[0m"
  if (((n + 1) % 8 == 0)); then line ""; fi
done

# ── 4. 24-bit truecolor ───────────────────────────────────────────────────────
sec "4. 24-bit truecolor — RGB ramp (red gradient)"
for r in $(seq 0 32 255); do
  emit "${ESC}[38;2;${r};64;128m█${ESC}[0m"
done
line "  (red 0..255 step 32, fg)"

sec "4b. 24-bit truecolor — primaries + secondaries"
rgb() { emit "${ESC}[38;2;$1;$2;$3m$4${ESC}[0m "; }
rgb 255 0 0   "red   "
rgb 0 255 0   "green "
rgb 0 0 255   "blue  "
rgb 255 255 0 "yellow"
rgb 255 0 255 "magent"
rgb 0 255 255 "cyan  "
rgb 255 255 255 "white "
rgb 128 128 128 "gray  "
line ""

sec "4c. 24-bit truecolor — bg ramp + colored fg on colored bg"
for g in $(seq 0 16 255); do
  emit "${ESC}[48;2;0;${g};64m ${ESC}[38;2;255;255;255m.${ESC}[0m"
done
line "  (green bg 0..255, white fg dots)"

# ── 5. Individual attributes ──────────────────────────────────────────────────
sec "5. individual attributes"
emit "${ESC}[1mbold${ESC}[0m  "
emit "${ESC}[2mdim${ESC}[0m  "
emit "${ESC}[3mitalic${ESC}[0m  "
emit "${ESC}[4munderline${ESC}[0m  "
emit "${ESC}[5mblink${ESC}[0m  "
emit "${ESC}[7mreverse${ESC}[0m  "
emit "${ESC}[8mhidden${ESC}[0m  "
emit "${ESC}[9mstrikethrough${ESC}[0m"
line ""

sec "5b. attributes over colored text"
emit "${ESC}[1;31mbold red${ESC}[0m  "
emit "${ESC}[4;32munderline green${ESC}[0m  "
emit "${ESC}[3;34mitalic blue${ESC}[0m  "
emit "${ESC}[7;33mreverse yellow${ESC}[0m  "
emit "${ESC}[9;35mstrike magenta${ESC}[0m"
line ""

# ── 6. Combined attributes ────────────────────────────────────────────────────
sec "6. combined attributes"
emit "${ESC}[1;4;31mbold+underline+red${ESC}[0m  "
emit "${ESC}[3;7;32mitalic+reverse+green${ESC}[0m  "
emit "${ESC}[1;3;4;34mbold+italic+underline+blue${ESC}[0m  "
emit "${ESC}[2;9;33mdim+strike+yellow${ESC}[0m  "
emit "${ESC}[1;4;38;5;208mbold+underline+256-orange${ESC}[0m  "
emit "${ESC}[1;4;38;2;255;100;0mbold+underline+truecolor-orange${ESC}[0m"
line ""

# ── 7. Reset behavior ─────────────────────────────────────────────────────────
sec "7. reset behavior — explicit reset vs new style"
emit "${ESC}[31mred ${ESC}[0mreset-to-default ${ESC}[32mgreen ${ESC}[33mno-reset-switch-to-yellow"
line ""
line "(line above ended with no reset; this line is plain default)"

sec "7b. partial resets — reset fg only (39), reset bg only (49)"
emit "${ESC}[31;44mred-on-blue ${ESC}[39mfg-reset-bg-stays-blue ${ESC}[49mboth-reset-now"
line ""

# ── 8. State carry across newlines ────────────────────────────────────────────
sec "8. state carry across newlines (no reset before \\n)"
emit "${ESC}[1;32mgreen-bold-line-one\n"
emit "green-bold-line-two (no reset was issued)\n"
emit "green-bold-line-three ${ESC}[0mnow reset"
line ""

# ── 9. Overlapping / nested styles on one line ────────────────────────────────
sec "9. overlapping styles on one line"
emit "plain ${ESC}[31mred ${ESC}[1mbold-red ${ESC}[4mbold-red-underline ${ESC}[31;0mreset-all plain"
line ""

# ── 10. Wide chars with color ─────────────────────────────────────────────────
sec "10. wide chars (emoji + CJK) with color"
emit "${ESC}[1;33m🦀 bold yellow crab${ESC}[0m  "
emit "${ESC}[32m緑色のテキスト green CJK${ESC}[0m  "
emit "${ESC}[4;34m下線 blue underlined CJK${ESC}[0m  "
emit "${ESC}[38;2;255;0;255m💜 magenta heart truecolor${ESC}[0m"
line ""

# ── 11. Empty / blank lines with active color state ───────────────────────────
sec "11. blank lines with active color state"
emit "${ESC}[32mgreen-then-blank-below:${ESC}[0m"
line ""
line "(above was a truly empty line; state should not leak)"
emit "${ESC}[1;31mred-bold-then-blank-below:${ESC}[0m"
line ""
emit "back to default after blank"
line ""

# ── 12. Long line for horizontal scroll / overflow ────────────────────────────
sec "12. long line — horizontal scroll + overflow indicator (…)"
emit "${ESC}[36m"
for i in $(seq 1 60); do emit "token$i "; done
emit "${ESC}[0m"
line ""
line "(60 colored tokens; pan with Shift-←/→, watch for … on both sides)"

# ── 13. Mixed fg/bg, partial resets ──────────────────────────────────────────
sec "13. mixed — colored fg over colored bg, then swap"
emit "${ESC}[38;5;45mfg-cyan ${ESC}[48;5;196mon-bg-red ${ESC}[38;5;226mfg-yellow-on-red ${ESC}[0mreset"
line ""

# ── 14. default fg with colored bg, and vice versa ───────────────────────────
sec "14. default fg + colored bg, then colored fg + default bg"
emit "${ESC}[48;5;17mdefault-fg on bg-blue-17${ESC}[0m   "
emit "${ESC}[38;5;51mfg-cyan on default-bg${ESC}[0m"
line ""

# ── 15. cursor-overlay candidates ─────────────────────────────────────────────
sec "15. cursor-overlay candidates — move the cursor onto these"
emit "${ESC}[31mRED${ESC}[0m ${ESC}[32mGREEN${ESC}[0m ${ESC}[34mBLUE${ESC}[0m ${ESC}[33mYELLOW${ESC}[0m ${ESC}[1;35mBOLD-MAGENTA${ESC}[0m ${ESC}[4;36mUNDERLINE-CYAN${ESC}[0m"
line ""
emit "${ESC}[38;2;255;128;0mtruecolor-orange${ESC}[0m ${ESC}[48;5;22m bg-22 ${ESC}[0m ${ESC}[7;31mreverse-red${ESC}[0m"
line ""
line "(cursor should fully replace the base style on the cell it lands on)"

line ""
emit "${ESC}[0m"
echo "wrote $out"
