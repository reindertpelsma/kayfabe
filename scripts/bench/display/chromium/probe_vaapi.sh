export DISPLAY=:0 XAUTHORITY=/home/ubuntu/.Xauthority
for _ in $(seq 40); do pgrep -x cinnamon >/dev/null && break; sleep 2; done
sleep 5
ls -la /dev/dri/ /dev/nvidia* 2>&1 | head -20
cat /sys/module/nvidia_drm/parameters/modeset
sudo dmesg > /tmp/dmesg.before
echo "== vainfo drm direct"; NVD_BACKEND=direct NVD_LOG=1 LIBVA_DRIVER_NAME=nvidia timeout 30 vainfo --display drm --device /dev/dri/renderD128 2>&1 | head -30
echo "== vainfo x11 default"; NVD_LOG=1 LIBVA_DRIVER_NAME=nvidia timeout 30 vainfo 2>&1 | head -20
echo "== ffmpeg cuda decode h264"; timeout 60 ffmpeg -hide_banner -loglevel info -hwaccel cuda -hwaccel_output_format cuda -i ~/kf/dl/h264.mp4 -f null - 2>&1 | tail -8
echo "== ffmpeg cuda decode av1"; timeout 60 ffmpeg -hide_banner -loglevel info -hwaccel cuda -i ~/kf/dl/av1.mp4 -f null - 2>&1 | tail -5
echo "== dmesg delta"; sudo dmesg | diff /tmp/dmesg.before - | grep '^>' | cut -c1-240 | head -30
