#include "nn/os.hpp"

// Plage: 0x0015EB80 .. 0x0015EB8C (12 octets, 100% matching)
// Importé par DllBattle.cro et le moteur principal sango-2.

BLUEORB_EXTERN_C u64 svcGetSystemTick_0015EB80(void) {
    register u32 r0 __asm__("r0");
    register u32 r1 __asm__("r1");
    __asm__ volatile("svc #0x28" : "=r"(r0), "=r"(r1));
    return ((u64)r1 << 32) | r0;
}

BLUEORB_EXTERN_C void nullsub_0015EB88(void) {
}
