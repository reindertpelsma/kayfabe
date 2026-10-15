#!/bin/bash
# kfps.sh FILE.ps1 [timeout] : run the file's text in the guest through the kfagent
timeout $(( ${2:-60} + 15 )) python3 /data/kfagent_server.py run "$(cat $1)" ${2:-60}
