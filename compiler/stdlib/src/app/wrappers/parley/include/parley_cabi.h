#ifndef PARLEY_CABI_H
#define PARLEY_CABI_H

#include <stdint.h>

double parley_measure(const uint8_t* text, int32_t len, double size);
int32_t parley_abi_version(void);

#endif
