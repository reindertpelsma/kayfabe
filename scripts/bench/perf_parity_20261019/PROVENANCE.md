# P1 measurement harness provenance

STATUS: RESEARCH, 2026-10-19.

CODE READ: Steel Nomad ETW/UI/analysis files derive from `claude/steelnomad-gap-20261018` commit `4f37bd12c`; tiny-submit/BPF/progress analysis files derive from `claude/submit-latency-20261018` commit `e97a35ffb`. Both are open repository measurement tools, not closed-binary analysis. The unchanged raw-COM probe uses the same work as the earlier measurement.

CODE READ: P1 adaptations confine guest files to the P1 directory; scheduled tasks and ETW/PresentMon sessions have a P1 name; PresentMon is only waited for using its recorded PID and creation time, with no global process kill. Original host shell harnesses with broad pkill and shared scratch paths are not copied. The host runner remains unimplemented; these files preserve the original probe and analysis formats.
