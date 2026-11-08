#ifndef CPU_CABI_H
#define CPU_CABI_H

#include <stdint.h>

int64_t cpu_render(int32_t w, int32_t h, const uint8_t* list, int32_t len, uint8_t* out);
int64_t cpu_register_font(int32_t id, const uint8_t* name, int32_t name_len, const uint8_t* data, int32_t data_len);
int64_t cpu_image_pending(void);
int64_t cpu_wait_images(int64_t timeout_ms);
int64_t cpu_abi_version(void);

#endif
