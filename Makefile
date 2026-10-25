# Tune Remote - the portable core and its tests (Linux / macOS).
#
#   make test     build and run test/core_test
#   make asan     the same under AddressSanitizer + UBSan
CXX      ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Werror
CORE     := core/tune.cpp core/midi_map.cpp core/apply.cpp core/settings.cpp core/log.cpp core/engine.cpp
HDRS     := $(wildcard core/*.h)

test: build/core_test
	./build/core_test

build/core_test: test/core_test.cpp $(CORE) $(HDRS)
	@mkdir -p build
	$(CXX) $(CXXFLAGS) -o $@ test/core_test.cpp $(CORE) -lpthread

asan: test/core_test.cpp $(CORE) $(HDRS)
	@mkdir -p build
	$(CXX) -std=c++17 -O1 -g -Wall -Wextra -fsanitize=address,undefined -fno-omit-frame-pointer -o build/core_test_asan test/core_test.cpp $(CORE) -lpthread
	./build/core_test_asan

clean:
	rm -rf build

.PHONY: test asan clean
