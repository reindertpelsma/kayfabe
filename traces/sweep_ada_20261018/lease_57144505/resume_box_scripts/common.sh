BASE=31b56f52a2f1adec4ef72c7a05ec2964dd857739
C5=89fc8a22291dd7bda69c3077ba829b1e934b11db
export WINVM_ROOT=/workspace/sweep-win-57144505 WINVM_RAM_MB=8192 WINVM_SMP=8
export PATH=/root/.cargo/bin:$PATH
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
