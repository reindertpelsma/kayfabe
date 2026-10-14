set -x
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq weston 2>&1 | tail -5
weston --version
ls /usr/lib/x86_64-linux-gnu/weston 2>/dev/null | head; ls /usr/lib/x86_64-linux-gnu/libweston-13 2>/dev/null | head
dpkg -l | grep -E "egl-wayland|libnvidia-egl" | awk '{print $2,$3}'; ls /usr/lib/x86_64-linux-gnu | grep -E "egl-wayland|nvidia-egl|libnvidia-vulkan|libEGL_nvidia" 
echo SETUP2_DONE
