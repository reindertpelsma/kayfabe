#!/bin/bash
# abl-sum.sh N — one-line verdict material for an ablation run (reads winprod/runN and boundary-kayfabe-N/qemu.log)
W=/var/lib/kf-windows-20261005; N=$1; O=$W/winprod/run$N; L=$W/boundary-kayfabe-$N/qemu.log
drop=$(sed -n 's/.*START run=[0-9]* drop=\[\([^]]*\)\].*/\1/p' $O/winprod.log | head -1)
c() { grep -a -c "$1" "$L" 2>/dev/null; }
nflags=$(python3 -I -c "import json;print(len(json.load(open('$O/command.json'))['flags']))" 2>/dev/null)
xid0=$(sed -n 's/.*xid_before=\([0-9]*\).*/\1/p' $O/winprod.log | head -1); xid1=$(sed -n 's/.*xid_lines=\([0-9]*\).*/\1/p' $O/winprod.log | tail -1)
echo "RUN $N drop=[$drop] flags_set=$nflags"
echo "  phases: $(grep -a -o 'PHASE [a-z-]* tdr_cycles=[0-9]*' $O/winprod.log | sed 's/PHASE //' | tr '\n' ' ')"
echo "  hold: $(grep -a 'HOLD ended' $O/winprod.log | sed 's/.*HOLD ended: //') ; tdr_cycles_final=$(c 'GSP phase Running -> Suspending')"
echo "  host: xid_before=$xid0 xid_after=$xid1 ; qemu.log: DEAD=$(c 'DEAD:') panicked=$(grep -a -c -i panicked $L) GSP_REFUSED_lines=$(c 'GSP REFUSED') GSP_cmd_REFUSED=$(c 'GSP command service REFUSED') STALL=$(c 'display: STALL ') halted=$(grep -a -c -i 'display.*halt' $L) failclosed=$(grep -a -c -i 'fail.\?closed' $L)"
echo "  guest: $(tr '\n' ';' < $O/ablate.txt 2>/dev/null | cut -c1-420)"
ref() { grep -a -o 'GSP REFUSED fn[0-9]*/0x[0-9a-f]*=0x[0-9a-f]*' "$1" | sort -u | sed 's/GSP REFUSED //'; }
B=$W/boundary-kayfabe-600/qemu.log
echo "  refused-set vs control600: only-here=[$(comm -13 <(ref $B) <(ref $L) | tr '\n' ' ')] only-control=[$(comm -23 <(ref $B) <(ref $L) | tr '\n' ' ')] total=$(ref $L | wc -l)"
ge=$O/guest-events.txt
echo "  guest-events: nvlddmkm=$(grep -a -c ' nvlddmkm ' $ge) dxgkrnl/display141_4101=$(grep -a -c -E ' (Display|dxgkrnl[a-z]*) (141|4101) ' $ge) bugcheck=$(grep -a -c -i -E 'bugcheck|Kernel-Power 41' $ge) ; screens(unique colours): signin=$(convert $O/after-signin.png -format %k info: 2>/dev/null) edge=$(convert $O/after-edge.png -format %k info: 2>/dev/null) final=$(convert $O/final-screen.png -format %k info: 2>/dev/null) hold=$(ls $O/hold-*.png 2>/dev/null | tail -1 | xargs -r -I{} convert {} -format %k info: 2>/dev/null)"
