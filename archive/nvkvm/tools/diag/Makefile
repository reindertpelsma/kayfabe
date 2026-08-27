CFLAGS ?= -O2 -g -Wall -Wextra

all: nvioctl_trace

nvioctl_trace: nvioctl_trace.c
	$(CC) $(CFLAGS) -o $@ $<

clean:
	rm -f nvioctl_trace

.PHONY: all clean
