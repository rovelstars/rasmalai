#ifndef VELLO_CABI_H
#define VELLO_CABI_H

#include <stdint.h>

int32_t vello_fill(int32_t w, int32_t h, uint8_t r, uint8_t g, uint8_t b, uint8_t a, uint8_t* out);
int32_t vello_render(int32_t w, int32_t h, const uint8_t* list, int32_t len, uint8_t* out);
int32_t vello_abi_version(void);

#endif
