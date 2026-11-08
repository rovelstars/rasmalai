# Media Wrapper

Native media backends for `@std/app/media`.

```text
media_cabi        C ABI loaded by the app
audio.rs          FFmpeg decode, bounded ring, ALSA or null sink
video.rs          FFmpeg decode, four-frame queue, RNXR frames
camera.rs         V4L2 capture, YUYV/RGB24 conversion, stub source
picker.rs         XDG portal dialog, headless stub
frame.rs          24-byte RNXR header plus RGBA8
```

Linux build dependencies:

```text
libasound2-dev libavcodec-dev libavformat-dev libavutil-dev
libswresample-dev libswscale-dev
```

The file dialog uses the XDG desktop portal through `rfd`; the camera uses
V4L2. Headless runs use:

```text
RNX_AUDIO=fast|null     audio sink mode
RNX_CAMERA=stub         synthetic camera frames
RNX_PICK_FILE=...       picker result, newline-separated for multi-select
RNX_TEST_VIDEO=...      video fixture for the wrapper test
```

`cargo test --release` covers WAV playback, video decode, the camera stub, the
picker stubs, and raw frame packing.
