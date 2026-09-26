# Out-of-tree build for kf_uvm_probe.
# Usage:
#   make NV_SYMVERS=/path/to/nvidia/Module.symvers
# Auto-discovers the nvidia Module.symvers if not given.
KDIR ?= /lib/modules/$(shell uname -r)/build
PWD  := $(shell pwd)

# Try to find nvidia's Module.symvers (has nvUvmInterface* CRCs)
ifndef NV_SYMVERS
NV_SYMVERS := $(shell grep -rl nvUvmInterfaceRegisterGpu /usr/src /var/lib/dkms 2>/dev/null | grep Module.symvers | head -1)
endif

all:
	@echo "KDIR=$(KDIR)"
	@echo "NV_SYMVERS=$(NV_SYMVERS)"
	@test -n "$(NV_SYMVERS)" || { echo "ERROR: NV_SYMVERS not found (nvidia Module.symvers with nvUvmInterface* CRCs)"; exit 1; }
	$(MAKE) -C $(KDIR) M=$(PWD) NV_SYMVERS=$(NV_SYMVERS) modules

clean:
	$(MAKE) -C $(KDIR) M=$(PWD) clean
