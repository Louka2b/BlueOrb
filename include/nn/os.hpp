#pragma once

#include "types.hpp"

namespace nn {
namespace os {

struct Tick {
    s64 value;
};

} // namespace os
} // namespace nn

// SVC 0x28: svcGetSystemTick (0x0015EB80 dans code.bin)
BLUEORB_EXTERN_C u64 svcGetSystemTick_0015EB80(void);
BLUEORB_EXTERN_C void sub_0015EB88(void);
