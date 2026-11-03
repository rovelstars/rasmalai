#ifndef HOST_CABI_H
#define HOST_CABI_H

#include <stdint.h>

typedef struct host_t* host;

host host_open(const uint8_t* title, int32_t title_len, int32_t width, int32_t height, int32_t backend);
int32_t host_pump(host h);
int32_t host_pump_timeout(host h, int32_t block);
int32_t host_wait(host h);
int32_t host_event_a(host h);
int32_t host_event_b(host h);
int32_t host_width(host h);
int32_t host_height(host h);
int32_t host_present(host h, const uint8_t* list, int32_t len, const int32_t* damage, int32_t count);
int32_t host_render_offscreen(int32_t w, int32_t h, const uint8_t* list, int32_t len, uint8_t* out);
void host_close(host h);
int32_t host_abi_version(void);

#endif
