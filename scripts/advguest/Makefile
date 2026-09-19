# SPDX-License-Identifier: GPL-2.0
# Out-of-tree kbuild for the adversarial guest module.
#
#   make            # build advguest.ko against the running kernel's headers
#   make KDIR=/path/to/linux-headers
#   make clean
#
# ⊘ The module is one .ko from two files (advguest.c + advguest.h). advguest.h is
# a plain include, not a compiled unit, so only advguest.o is listed.
obj-m += advguest.o

KDIR ?= /lib/modules/$(shell uname -r)/build
PWD  := $(shell pwd)

all:
	$(MAKE) -C $(KDIR) M=$(PWD) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean

.PHONY: all clean
