/* guard.c - one call into LV1 behind SEH. A fault inside LV1's callback (a bad pointer,
 * an ABI that changed) comes back as a return value instead of taking LV1 down.
 * Plain C on purpose: Rust has no __try, and nothing here needs unwinding. */
#include <windows.h>

typedef int (*lv1_fn)(void* core, void* arg);

int lv1s_guarded_call(lv1_fn fn, void* core, void* arg, int* rc) {
    __try {
        *rc = fn(core, arg);
        return 1;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        *rc = (int)GetExceptionCode();
        return 0;
    }
}
