// clock.h - the steady millisecond clock every part of Tune Remote uses, and a bounded
// thread join for teardown (LV1 calls DestroyController on its own thread, which must
// not wait long).
#pragma once
#include <stdint.h>
#include <thread>

namespace trw {
uint64_t now_ms();   // QueryPerformanceCounter based; never 0
// Joins t if it ends within ms; otherwise detaches it (its objects are never freed).
// True when it ended.
bool join_for(std::thread& t, unsigned ms);
}
