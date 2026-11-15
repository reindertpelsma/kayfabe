# manual release rehearsal env (mirrors vast_payload.py main())
export R=/workspace/kf-ci-src P=/root/prov B=/workspace/bench
export TAG=cirelgate0001 SHA=a76d139790ab2ef126d4abd26b9237e887c5af15
export KF_PROVISION_PROFILE=ci KAYFABE_BRANCH=kf-ci-approved KAYFABE_REPO=$R BENCH_DIR=$B KF_DEVICE=kf3 KF_REQUIRE_ORACLES=1
export CARGO_TARGET_DIR=$R/target CARGO_BUILD_JOBS=8 CARGO_TERM_QUIET=true CARGO_TERM_COLOR=never CARGO_TERM_PROGRESS_WHEN=never
export PATH=$HOME/.cargo/bin:$PATH
