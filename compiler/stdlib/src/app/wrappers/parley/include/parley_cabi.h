#ifndef PARLEY_CABI_H
#define PARLEY_CABI_H

#include <stdint.h>

double parley_measure(const uint8_t* text, int32_t len, double size);
int32_t parley_clusters(const uint8_t* text, int32_t len, double size, uint8_t* out, int32_t cap);
int32_t parley_abi_version(void);

#endif
