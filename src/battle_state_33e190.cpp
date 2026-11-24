#include "types.hpp"

// Plage: 0x0033E190 .. 0x0033E198 (8 octets, 100% matching)
// Importé par DllBattle.cro

struct BattleStateObj33E190 {
    u8 pad_00[0x32];
    u16 param_32; // +0x32
};

BLUEORB_EXTERN_C u16 BattleObj_GetParam32_0033E190(const BattleStateObj33E190* self) {
    return self->param_32;
}
