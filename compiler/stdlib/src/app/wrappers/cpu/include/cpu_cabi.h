#ifndef CPU_CABI_H
#define CPU_CABI_H

#include <stdint.h>

int32_t cpu_render(int32_t w, int32_t h, const uint8_t* list, int32_t len, uint8_t* out);
int32_t cpu_abi_version(void);

#endif
