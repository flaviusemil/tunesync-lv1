# Tune Remote - the portable parts and their tests (Linux / macOS). The Windows DLL and
# installer: tools/xbuild.sh and tools/make-installer.sh.
#
#   make test     build and run test/core_test and test/net_test
#   make asan     the same under AddressSanitizer + UBSan
CXX      ?= c++
CXXFLAGS ?= -std=c++17 -O2 -Wall -Wextra -Werror
CORE     := core/tune.cpp core/midi_map.cpp core/apply.cpp core/settings.cpp core/log.cpp core/engine.cpp
NET      := net/rtp_codec.cpp net/rtp_session.cpp
HDRS     := $(wildcard core/*.h net/*.h)
ASANFLAGS := -std=c++17 -O1 -g -Wall -Wextra -fsanitize=address,undefined -fno-omit-frame-pointer

all: test build/rtpsend

test: build/core_test build/net_test
	./build/core_test
	./build/net_test

build/core_test: test/core_test.cpp $(CORE) $(HDRS)
	@mkdir -p build
	$(CXX) $(CXXFLAGS) -o $@ test/core_test.cpp $(CORE) -lpthread

build/net_test: test/net_test.cpp $(NET) $(HDRS)
	@mkdir -p build
	$(CXX) $(CXXFLAGS) -o $@ test/net_test.cpp $(NET) -lpthread

# A stand-in playback rig for tests: rtpsend <host> 5004 key A cc 20 3
build/rtpsend: tools/rtpsend.cpp $(NET) $(HDRS)
	@mkdir -p build
	$(CXX) $(CXXFLAGS) -o $@ tools/rtpsend.cpp $(NET) -lpthread

asan: test/core_test.cpp test/net_test.cpp $(CORE) $(NET) $(HDRS)
	@mkdir -p build
	$(CXX) $(ASANFLAGS) -o build/core_test_asan test/core_test.cpp $(CORE) -lpthread
	$(CXX) $(ASANFLAGS) -o build/net_test_asan test/net_test.cpp $(NET) -lpthread
	./build/core_test_asan
	./build/net_test_asan

clean:
	rm -rf build

.PHONY: all test asan clean
