BASE=7f22d18dfff8083345cdb54a4666018dca845384
CAND=aa8fdea9cb0e00fd4e5ec148461d1e805d308e0c
C5=f8f9d1a5f0aae58fc8ccf0da740cf12682cd284e
export WINVM_ROOT=/workspace/bwr-win WINVM_RAM_MB=8192 WINVM_SMP=8
export PATH=/root/.cargo/bin:$PATH
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
