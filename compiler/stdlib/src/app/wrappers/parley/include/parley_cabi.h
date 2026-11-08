#ifndef PARLEY_CABI_H
#define PARLEY_CABI_H

#include <stdint.h>

double parley_measure(const uint8_t* text, int32_t len, double size);
double parley_measure_styled(
    const uint8_t* text,
    int32_t len,
    double size,
    int32_t weight,
    int32_t italic,
    double letter_spacing
);
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
int64_t parley_layout_styled(
    const uint8_t* text,
    int32_t len,
    double size,
    double max_width,
    int32_t max_lines,
    int32_t wrap,
    int32_t ellipsis,
    int32_t weight,
    int32_t italic,
    double line_height,
    double letter_spacing,
    uint8_t* out,
    int32_t cap
);
/*
 * A rich span is exactly 32 bytes:
 *   uint32_t start;     start character offset
 *   uint32_t end;       end character offset
 *   int32_t weight;     CSS weight 100..900
 *   uint32_t italic;    nonzero for italic
 *   double tracking;    extra advance per character
 *   uint32_t style;     low 8 bits family id, bits 8-9 decoration
 *   uint32_t reserved;  ignored, write zero
 *
 * Family ids:
 *   0 sans-serif, 1 serif, 2 monospace, 3 cursive, 4 fantasy, 5 system-ui,
 *   6 ui-serif, 7 ui-sans-serif, 8 ui-monospace, 9 ui-rounded, 10 emoji,
 *   11 math. Unknown ids fall back to sans-serif.
 *
 * Decoration bits in style:
 *   bit 8 set: underline
 *   bit 9 set: strikethrough
 *
 * Text not covered by a span uses sans-serif with no decoration.
 */
typedef struct parley_span {
    uint32_t start;
    uint32_t end;
    int32_t weight;
    uint32_t italic;
    double tracking;
    uint32_t style;
    uint32_t reserved;
} parley_span;
typedef char parley_span_size_assert[(sizeof(parley_span) == 32) ? 1 : -1];
/*
 * Rich layout output:
 *   header 16 bytes: rows u32, runs u32, reserved u64
 *   rows 24 bytes: start u32, end u32, flags u32, width f32, height f32, baseline f32
 *   runs 32 bytes: row u32, start u32, end u32, flags u32, x f32, advance f32,
 *                  baseline f32, reserved f32
 */
int64_t parley_layout_rich(
    const uint8_t* text,
    int32_t len,
    double size,
    double max_width,
    int32_t max_lines,
    int32_t wrap,
    int32_t ellipsis,
    double line_height,
    const uint8_t* spans,
    int32_t span_count,
    uint8_t* out,
    int32_t cap
);
int64_t parley_clusters(const uint8_t* text, int32_t len, double size, uint8_t* out, int32_t cap);
/* Returns 6 for the 32-byte span format. */
int64_t parley_abi_version(void);

#endif
