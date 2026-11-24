/*
 * BlueOrb — Remplacements C natifs (Overrides AOT pour 3dsrecomp)
 *
 * Chaque fonction déclarée ici avec RECOMP_OVERRIDE(adresse) ou
 * RECOMP_OVERRIDE_IN(ModuleCRO, offset) remplace directement la fonction ARM11
 * correspondante de Pokémon Saphir Alpha par du code C natif compilé pour
 * votre processeur hôte (Apple Silicon ARM64 / x86_64).
 *
 * Exemple :
 *
 * #include "overrides.h"
 *
 * RECOMP_OVERRIDE(0x0033E190) {
 *     uint32_t self = ctx->r[0];
 *     ctx->r[0] = mem_read16(ctx, self + 0x32);
 *     RETURN_TO(ctx->r[14]);
 * }
 */

#include "overrides.h"
