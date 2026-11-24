#include "types.hpp"

// Plage: 0x0034B1EC .. 0x0034B1F8 (12 octets, 100% matching)
// Importé par DllBattle.cro

struct BattleFlagObj34B1EC {
    u8 pad_00[0x24];
    bool8 flag_24; // +0x24
};

BLUEORB_EXTERN_C void BattleObj_DisableFlag24_0034B1EC(BattleFlagObj34B1EC* self) {
    self->flag_24 = 0;
}
