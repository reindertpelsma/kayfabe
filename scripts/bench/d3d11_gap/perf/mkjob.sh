#!/bin/bash
# mkjob.sh TEMPLATE OUT KEY=VAL... : fill @KEY@ placeholders
T=$1; O=$2; shift 2; cp $T $O; for kv in "$@"; do k=${kv%%=*}; v=${kv#*=}; sed -i "s|@$k@|$v|g" $O; done
