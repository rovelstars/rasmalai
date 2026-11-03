#ifndef PARLEY_CABI_H
#define PARLEY_CABI_H

#include <stdint.h>

double parley_measure(const uint8_t* text, int32_t len, double size);
int64_t parley_grapheme_next(const uint8_t* text, int32_t len, int32_t at);
int64_t parley_grapheme_prev(const uint8_t* text, int32_t len, int32_t at);
int64_t parley_layout(
    const uint8_t* text,
    int32_t len,
    double size,
    double max_width,
    int32_t max_lines,
    int32_t wrap,
    int32_t ellipsis,
    uint8_t* out,
    int32_t cap
);
int64_t parley_clusters(const uint8_t* text, int32_t len, double size, uint8_t* out, int32_t cap);
int64_t parley_abi_version(void);

#endif
