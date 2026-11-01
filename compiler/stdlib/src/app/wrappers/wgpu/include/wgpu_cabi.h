#ifndef WGPU_CABI_H
#define WGPU_CABI_H

#include <stdint.h>

typedef struct wgpu_instance_t* wgpu_instance;

wgpu_instance wgpu_instance_create(void);
void wgpu_instance_destroy(wgpu_instance p);
int32_t wgpu_adapter_count(wgpu_instance p);
int32_t wgpu_adapter_name(wgpu_instance p, int32_t index, uint8_t* out, int32_t len);
int32_t wgpu_clear_rgba(int32_t w, int32_t h, uint8_t r, uint8_t g, uint8_t b, uint8_t a, uint8_t* out);
int32_t wgpu_triangle(int32_t w, int32_t h, uint8_t* out);
int32_t wgpu_abi_version(void);

#endif
