#include "types.hpp"

// Plage: 0x0034B0C0 .. 0x0034B0CC (12 octets, 100% matching)
// Importé par DllBattle.cro

struct BattleFlagObj34B0C0 {
    u8 pad_00[0x24];
    bool8 flag_24; // +0x24
};

BLUEORB_EXTERN_C void BattleObj_EnableFlag24_0034B0C0(BattleFlagObj34B0C0* self) {
    self->flag_24 = 1;
}
