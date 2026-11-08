#ifndef MEDIA_CABI_H
#define MEDIA_CABI_H

#include <stdint.h>

typedef struct media_handle* media;

int64_t media_abi_version(void);
int64_t media_handle_address(media h);

media media_audio_open(const uint8_t* path, int32_t len);
void media_audio_close(media h);
int64_t media_audio_play(media h);
int64_t media_audio_pause(media h);
int64_t media_audio_stop(media h);
int64_t media_audio_seek(media h, int64_t ms);
int64_t media_audio_position(media h);
int64_t media_audio_duration(media h);
int64_t media_audio_channels(media h);
int64_t media_audio_rate(media h);
int64_t media_audio_volume(media h, int32_t percent);
int64_t media_audio_state(media h);
int64_t media_audio_pending(media h);

media media_video_open(const uint8_t* path, int32_t len);
void media_video_close(media h);
int64_t media_video_info(media h, int32_t lane);
int64_t media_video_next(media h, uint8_t* out, int32_t cap);
int64_t media_video_seek(media h, int64_t ms);
int64_t media_video_pending(media h);

media media_camera_open(const uint8_t* device, int32_t len, int32_t w, int32_t h);
void media_camera_close(media h);
int64_t media_camera_info(media h, int32_t lane);
int64_t media_camera_next(media h, uint8_t* out, int32_t cap);
int64_t media_camera_pending(media h);

int64_t media_pick_file(const uint8_t* title, int32_t len, const uint8_t* filters, int32_t filters_len, uint8_t* out, int32_t cap);
int64_t media_pick_files(const uint8_t* title, int32_t len, const uint8_t* filters, int32_t filters_len, uint8_t* out, int32_t cap);

#endif
