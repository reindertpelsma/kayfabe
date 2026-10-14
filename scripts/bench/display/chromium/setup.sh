set -x
cd ~/kf
sudo systemctl stop unattended-upgrades 2>/dev/null; sudo pkill -f unattended 2>/dev/null
sudo apt-get update -qq 2>&1 | tail -2
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq python3-websocket ffmpeg vulkan-tools mesa-utils vainfo nvidia-vaapi-driver unzip x11-utils xdotool ./dl/chrome.deb 2>&1 | tail -15
mkdir -p electron && unzip -q -o dl/electron.zip -d electron && ls electron | head -5
google-chrome-stable --version; ./electron/electron --version 2>&1 | head -2
vulkaninfo --summary 2>&1 | grep -E "deviceName|driverName|apiVersion|ERROR" | head -8
vainfo 2>&1 | head -6
ls /usr/lib/x86_64-linux-gnu/ | grep -E "nvcuvid|nvidia-encode|libcuda|nvidia-vaapi|nvidia_drv" ; ls /usr/lib/x86_64-linux-gnu/dri | grep -i nvidia
ls /usr/share/vulkan/icd.d /etc/vulkan/icd.d 2>&1 | head -20
python3 -c "import websocket;print('websocket ok',websocket.__version__)"
df -h / | tail -1
echo SETUP_DONE
