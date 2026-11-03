#ifndef TAFFY_CABI_H
#define TAFFY_CABI_H

#include <stdint.h>

int64_t taffy_layout(const uint8_t* nodes, int32_t count, double root_w, double root_h, uint8_t* out);
int64_t taffy_abi_version(void);

#endif
