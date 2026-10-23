BASE=ac21aef93cc7a8bb1149fecd98f8404e6b7e20d7
C5=89fc8a22291dd7bda69c3077ba829b1e934b11db
export WINVM_ROOT=/workspace/sweep-win-57296386 WINVM_RAM_MB=8192 WINVM_SMP=8
export PATH=/root/.cargo/bin:$PATH
W=scripts/bench/windows/win_vm.sh
VM=$WINVM_ROOT/kfwin
export WINVM_KF3_FB_MB=8192
export S4_SOURCE=ac21aef93cc7a8bb1149fecd98f8404e6b7e20d7

export WINVM_READINESS_PROFILE=/root/prov/box_readiness.json WINVM_READINESS_LEASE=/root/s2/lease.json
