# guest side: Chrome as a Wayland client (--ozone-platform=wayland) in a nested weston on the Cinnamon Xorg session.
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
sleep 10
mkdir -p ~/out/$CFG_NAME
export XDG_RUNTIME_DIR=/run/user/$(id -u)
weston --backend=x11-backend.so --width=1650 --height=950 --socket=wl-kf > ~/out/$CFG_NAME/weston.log 2>&1 &
WPID=$!
sleep 6
echo "WESTON pid=$WPID alive=$(kill -0 $WPID 2>/dev/null && echo yes || echo no)"; tail -5 ~/out/$CFG_NAME/weston.log
export WAYLAND_DISPLAY=wl-kf
python3 ~/kf/cdp_load.py --kind "$CFG_KIND" --name "$CFG_NAME" --minutes "${CFG_MIN:-11}" --outdir ~/out/$CFG_NAME \
    --flags "--ozone-platform=wayland ${CFG_FLAGS:-}" ${CFG_HASH:+--hash "$CFG_HASH"} ${CFG_EXTRA:-}
echo "CDP_LOAD_RC=$?"
kill $WPID 2>/dev/null
