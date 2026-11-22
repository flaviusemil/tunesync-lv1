/* bcryptprimitives.c - TEST ONLY, for running TuneSync's Windows binaries under Wine 8:
 * Rust's standard library imports ProcessPrng from bcryptprimitives.dll (every Windows 10
 * and 11 has it; Wine 8 does not). Never shipped. */
#include <windows.h>
#include <bcrypt.h>

__declspec(dllexport) BOOL WINAPI ProcessPrng(PBYTE data, SIZE_T len) {
    return BCryptGenRandom(NULL, data, (ULONG)len, BCRYPT_USE_SYSTEM_PREFERRED_RNG) >= 0;
}
