BASE=6e3fd3fd6d809ae2c2a3504cd8be1ecd29b2b31b
C5=89fc8a22291dd7bda69c3077ba829b1e934b11db
export WINVM_ROOT=/workspace/bw-capc-validation WINVM_RAM_MB=8192 WINVM_SMP=6
export PATH=/root/.cargo/bin:$PATH
W=/workspace/bw-readiness/scripts/bench/windows/win_vm.sh
export WINVM_SSH_CONNECT_TIMEOUT=100
VM=$WINVM_ROOT/kfwin
