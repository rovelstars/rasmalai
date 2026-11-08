#ifndef HOST_CABI_H
#define HOST_CABI_H

#include <stdint.h>

typedef struct host_t* host;

host host_open(const uint8_t* title, int32_t title_len, int32_t width, int32_t height, int32_t backend);
int64_t host_pump(host h);
int64_t host_pump_timeout(host h, int32_t block);
int64_t host_wait(host h);
int64_t host_event_a(host h);
int64_t host_event_b(host h);
int64_t host_event_c(host h);
int64_t host_width(host h);
int64_t host_generation(host h);
int64_t host_height(host h);
int64_t host_present(host h, const uint8_t* list, int32_t len, const int32_t* damage, int32_t count);
int64_t host_render_offscreen(int32_t w, int32_t h, const uint8_t* list, int32_t len, uint8_t* out);
void host_close(host h);
int64_t host_abi_version(void);
int64_t host_register_font(int32_t id, const uint8_t* name, int32_t name_len, const uint8_t* data, int32_t data_len);
int64_t host_address(host h);
void* host_surface_socket_open(const uint8_t* path, int32_t len, uint32_t window_id, uint32_t width, uint32_t height);
int64_t host_surface_socket_commit(void* h, const uint8_t* list, int32_t len, const uint8_t* packet, int32_t packet_len, const int32_t* damage, int32_t count);
int64_t host_surface_socket_id(void* h);
void host_surface_socket_close(void* h);
int64_t host_surface_socket_test_start(const uint8_t* path, int32_t len);
int64_t host_surface_socket_test_frames(const uint8_t* path, int32_t len);
int64_t host_surface_socket_test_last_id(const uint8_t* path, int32_t len);
int64_t host_surface_socket_test_last_list(const uint8_t* path, int32_t len);
int64_t host_surface_socket_test_last_packet(const uint8_t* path, int32_t len);
int64_t host_open_window(host h, const uint8_t* title, int32_t len, uint32_t width, uint32_t height);
int64_t host_window_count(host h);
int64_t host_window_width(host h, uint32_t id);
int64_t host_window_height(host h, uint32_t id);
int64_t host_window_generation(host h, uint32_t id);
int64_t host_event_window(host h);
int64_t host_present_window(host h, uint32_t id, const uint8_t* list, int32_t len, const int32_t* damage, int32_t count);
int64_t host_frame_packet(host h, const uint8_t* data, int32_t len);
int64_t host_a11y(host h, const uint8_t* data, int32_t len);
int64_t host_pointer_capture(host h, int32_t mode);
int64_t host_ime_enable(host h, int32_t x, int32_t y, int32_t w, int32_t hgt);
void host_ime_disable(host h);
int64_t host_text_byte(host h, int32_t index);
int64_t host_clipboard_set(host h, const uint8_t* text, int32_t len);
int64_t host_clipboard_get(host h, uint8_t* out, int32_t cap);
int64_t host_clipboard_generation(host h);
int64_t host_set_cursor(host h, int32_t shape);

#endif
