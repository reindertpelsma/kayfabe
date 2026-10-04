# Bench-only instrument (see kfdsw_probe.c); built ON the bench host by dispsw_trace.sh.
obj-m := kfdsw_probe.o
KDIR ?= /lib/modules/$(shell uname -r)/build
all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules
clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean
