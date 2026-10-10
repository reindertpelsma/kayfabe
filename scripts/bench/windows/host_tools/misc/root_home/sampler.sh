#!/bin/bash
while true; do /root/va_probe.sh >> /root/va_log.txt 2>&1; sleep 30; done
