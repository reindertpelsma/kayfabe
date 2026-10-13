# SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
# GPU-free tests of the platform independent library. Build output goes to $(OUT) (default /tmp).
CC ?= gcc
OUT ?= /tmp/kfwc-build
LIB := ../lib
SRC := $(LIB)/kfwc_record.c $(LIB)/kfwc_elem.c $(LIB)/kfwc_discover.c $(LIB)/kfwc_engine.c
CFLAGS := -std=c99 -O1 -g -Wall -Wextra -Werror -Wshadow -Wno-unused-function -I$(LIB)
SAN ?= -fsanitize=address,undefined -fno-sanitize-recover=undefined

.PHONY: test clean
test: $(OUT)/test_main
	$(OUT)/test_main

$(OUT)/test_main: test_main.c synth.h $(SRC) $(wildcard $(LIB)/*.h)
	mkdir -p $(OUT)
	$(CC) $(CFLAGS) $(SAN) test_main.c $(SRC) -o $@

clean:
	rm -rf $(OUT)

$(OUT)/test_kayfabe_roundtrip: test_kayfabe_roundtrip.c synth.h $(SRC) $(wildcard $(LIB)/*.h)
	mkdir -p $(OUT)
	$(CC) $(CFLAGS) $(SAN) test_kayfabe_roundtrip.c $(SRC) -o $@
