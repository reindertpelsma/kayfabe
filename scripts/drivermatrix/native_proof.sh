# Optional, source-pinned native proof stage for sweep.sh. No cached success is an admission:
# routes and harvesting belong to the current host. A failed prior proof requires explicit retry.
native_proof() {
    [ -n "${SWEEP_NATIVE_PROOF:-}" ] || return 0
    local id="native_proof:$H" rc log
    if [ "$DRY" = 1 ]; then
        echo "PLAN row=$id: $SWEEP_NATIVE_PROOF (current host; before any guest)"
        return 0
    fi
    if failed_before "$id"; then
        emit "NATIVE_PROOF host=$H rc=$(row_rc "$id") resumed_failure=1 arch=$ARCH"
        return 1
    fi
    # Only scripts in this revision may participate in this optional stage.
    if [[ ! "$SWEEP_NATIVE_PROOF" =~ ^scripts/[A-Za-z0-9_./-]+\.sh$ ]] || \
       [[ "$SWEEP_NATIVE_PROOF" == *..* ]]; then
        emit "NATIVE_PROOF host=$H rc=2 invalid_script=1 arch=$ARCH"
        return 1
    fi
    row_start "$id"; log=$(rowlog "$id")
    step "${T_NATIVE_PROOF:-900}" "$log" bash "$REPO/$SWEEP_NATIVE_PROOF" "$H" "$ARCH" "$REPO" "$DIR"; rc=$?
    emit "NATIVE_PROOF host=$H rc=$rc arch=$ARCH"
    row_end "$id" "$rc"
    [ "$rc" = 0 ]
}
