#!/bin/bash
# usage: as_user.sh <cmd...>  : run as kfrootless with a clean env (no root HOME/PATH leakage)
exec setpriv --reuid=kfrootless --regid=kfrootless --groups=kvm env -i HOME=/home/kfrootless USER=kfrootless LOGNAME=kfrootless PATH=/usr/local/bin:/usr/bin:/bin SHELL=/bin/bash LANG=C.UTF-8 "$@"
