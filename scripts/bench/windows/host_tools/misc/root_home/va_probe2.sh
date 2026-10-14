#!/bin/bash
/root/va_probe.sh | tr -d "\n"
echo -n " "
python3 /root/mapva.py 2>/dev/null || echo mapva_used=ERR
