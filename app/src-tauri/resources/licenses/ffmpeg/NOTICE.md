# FFmpeg sidecar notice

回声记忆包含一个独立运行、可由用户替换的 FFmpeg 7.1.1 arm64 可执行文件，仅用于本地音频预处理。

- 上游项目：https://ffmpeg.org/
- 源码：https://ffmpeg.org/releases/ffmpeg-7.1.1.tar.xz
- 源码 SHA-256：`733984395e0dbbe5c046abda2dc49a5544e7e0e1e2366bba849222ae9e3a03b1`
- 许可证：LGPL-2.1-or-later
- 构建平台：macOS arm64，Apple clang 17.0.0
- 产物架构：Mach-O 64-bit executable arm64

构建配置：

```text
./configure
--arch=arm64
--cc=/usr/bin/clang
--disable-everything
--disable-autodetect
--disable-doc
--disable-debug
--disable-network
--disable-shared
--enable-static
--disable-programs
--enable-ffmpeg
--enable-avcodec
--enable-avformat
--enable-avfilter
--enable-swresample
--enable-protocol=file
--enable-demuxer=mov,mp3,wav,aac,flac,ogg,matroska
--enable-decoder=aac,alac,mp3,mp3float,pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,flac,opus,vorbis
--enable-parser=aac,mpegaudio,flac,opus,vorbis
--enable-encoder=pcm_s16le
--enable-muxer=wav
--enable-filter=aresample,aformat,highpass,lowpass,loudnorm,pan,volume
--enable-small
```

未启用 GPL、nonfree、网络协议或第三方编解码库。应用通过子进程调用 FFmpeg，不链接 FFmpeg 库；用户可用兼容的 FFmpeg 可执行文件替换 `Contents/Resources/ffmpeg`。
