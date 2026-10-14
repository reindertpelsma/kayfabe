# guest side of one chromium_lane.sh run; CFG_KIND (chrome|electron) CFG_NAME CFG_MIN CFG_FLAGS CFG_HASH CFG_ENV CFG_EXTRA
export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
sleep 10
echo "CINNAMON $(pgrep -c -x cinnamon) XORG $(pgrep -c -x Xorg) $(date -Is)"
glxinfo -B 2>&1 | grep -E 'OpenGL renderer|OpenGL version' | sed 's/^/GLX /'
mkdir -p ~/out/$CFG_NAME
[ -n "${CFG_ENV:-}" ] && export $CFG_ENV
echo "RUN kind=$CFG_KIND name=$CFG_NAME min=$CFG_MIN flags=[$CFG_FLAGS] hash=[${CFG_HASH:-}] env=[${CFG_ENV:-}] extra=[${CFG_EXTRA:-}]"
python3 ~/kf/cdp_load.py --kind "$CFG_KIND" --name "$CFG_NAME" --minutes "${CFG_MIN:-11}" --outdir ~/out/$CFG_NAME \
    --flags "${CFG_FLAGS:-}" --hash "${CFG_HASH:-}" ${CFG_EXTRA:-}
echo "CDP_LOAD_RC=$?"
