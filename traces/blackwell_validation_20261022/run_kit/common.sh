BASE=8f3a9bb3a5d652b3f536ca015bf95735ec588924
C5=89fc8a22291dd7bda69c3077ba829b1e934b11db
export WINVM_ROOT=/workspace/bw-capc-validation WINVM_RAM_MB=8192 WINVM_SMP=8
export PATH=/root/.cargo/bin:$PATH
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
