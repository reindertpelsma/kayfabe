r() { local o=""; for c in "$@"; do for i in 1 2 3; do o="$o $c:$i"; done; done; echo "$o"; }
G1="$(r c_ok c_badclass c_badsw c_hw_rebind c_hw_two r0) c_hwsub0:1 c_hwsub1:1 c_hwsub2:1 c_hwsub3:1 c_hwsub4:1"
G2="$(r s1 s1r s2 s2r)"
G3="$(r s2b s2br s2c5 s2c5r s2c7 s2c7r s2d s2dr)"
G4="$(r s2e5 s2e7 s2e5r s2e7r enc) s2e_hw0n:1 s2e_hw0e:1 s2e_hw1n:1 s2e_hw1e:1 s2e_hw2n:1 s2e_hw2e:1 s2e_hw3n:1 s2e_hw3e:1 s2e_hw4n:1 s2e_hw4e:1"
G5="$(r s3_6 s3_5 s3_7 s3_6_3d s3_5_3d s3_7_3d)"
echo "cd /data/claude-setobj/src && nohup bash queue.sh b \"$G1\" \"$G2\" \"$G3\" \"$G4\" \"$G5\" > /dev/null 2>&1 < /dev/null &"
