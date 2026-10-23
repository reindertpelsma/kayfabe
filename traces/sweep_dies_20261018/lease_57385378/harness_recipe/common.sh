BASE=98b599bbfc31784bcbed6b42545562094ea6eeb0
C5=89fc8a22291dd7bda69c3077ba829b1e934b11db
export WINVM_ROOT=/workspace/sweep-win-57385378 WINVM_RAM_MB=8192 WINVM_SMP=8
export PATH=/root/.cargo/bin:$PATH
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
export WINVM_KF3_FB_MB=8192
export S4_SOURCE=98b599bbfc31784bcbed6b42545562094ea6eeb0

export WINVM_READINESS_PROFILE=/root/prov/box_readiness.json WINVM_READINESS_LEASE=/root/s2/lease.json
