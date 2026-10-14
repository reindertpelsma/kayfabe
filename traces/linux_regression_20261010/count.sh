#!/bin/bash
# usage: count.sh label files...
l=$1; shift
n=0; for f in "$@"; do [ -f "$f" ] && n=$((n+1)); done
c() { cat "$@" 2>/dev/null | grep -aEc "$P"; }
printf "%-22s files=%-3s" "$l" "$n"
for P in "HELD BY HOST" "HELD-BY-OURSELVES" "batch fallback" "micro reservation.*refused" "CHANNEL BIRTH REFUSED|PRIVILEGED CHANNEL REFUSED|CHANNEL CLASS REFUSED|CUDA THREAD REFUSED" "DEAD|POISONED" "ring REFUSED|REFUSED segment"; do
  printf " [%s]=%s" "$(echo "$P"|cut -c1-18)" "$(cat "$@" 2>/dev/null | grep -aEc "$P")"
done; echo
