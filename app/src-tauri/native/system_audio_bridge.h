#ifndef ECHO_MEMORY_SYSTEM_AUDIO_BRIDGE_H
#define ECHO_MEMORY_SYSTEM_AUDIO_BRIDGE_H

#ifdef __cplusplus
extern "C" {
#endif

enum EchoSystemAudioResult {
    ECHO_SYSTEM_AUDIO_OK = 0,
    ECHO_SYSTEM_AUDIO_INVALID_ARGUMENT = -1,
    ECHO_SYSTEM_AUDIO_PERMISSION_DENIED = -2,
    ECHO_SYSTEM_AUDIO_UNSUPPORTED = -3,
    ECHO_SYSTEM_AUDIO_BUSY = -4,
    ECHO_SYSTEM_AUDIO_NOT_ACTIVE = -5,
    ECHO_SYSTEM_AUDIO_AUDIO_ERROR = -6,
    ECHO_SYSTEM_AUDIO_PERMISSION_TIMEOUT = -7,
};

enum EchoSystemAudioPermission {
    ECHO_SYSTEM_AUDIO_PERMISSION_UNDETERMINED = 0,
    ECHO_SYSTEM_AUDIO_PERMISSION_DENIED_STATUS = 1,
    ECHO_SYSTEM_AUDIO_PERMISSION_GRANTED = 2,
    ECHO_SYSTEM_AUDIO_PERMISSION_UNSUPPORTED_STATUS = 3,
};

/* 样本回调：SCK 音频队列线程调用，交来的永远是 IEEE Float32 交错样本。
 * 回调必须快速返回；文件落盘、格式转换由 Rust 侧负责。
 * 注意：channels 为声道数（交错存储），frame_count 为帧数。 */
typedef void (*echo_system_audio_sample_callback)(const float *interleaved_samples,
                                                  int frame_count,
                                                  int channels,
                                                  unsigned int sample_rate);

void echo_system_audio_set_sample_callback(echo_system_audio_sample_callback callback);

int echo_system_audio_permission_status(void);

/* 与麦克风一致：必须在非主线程调用，内部到主队列发起授权请求再阻塞等待。
 * 返回 1 已授权、0 已拒绝、-1 平台不支持、-2 等待超时。 */
int echo_system_audio_request_permission(void);

/* 同步枚举可按应用过滤的系统声音来源。bundle_id 只在回调期间有效。 */
typedef void (*echo_system_audio_application_callback)(const char *bundle_id,
                                                       const char *name,
                                                       int process_id,
                                                       void *context);
int echo_system_audio_list_applications(echo_system_audio_application_callback callback,
                                        void *context);

/* application_bundle_id 为 NULL 或空串时录制整台 Mac 的系统声音。 */
int echo_system_audio_start(const char *application_bundle_id);
int echo_system_audio_stop(void);
int echo_system_audio_cancel(void);
int echo_system_audio_is_recording(void);

/* 音频流因系统错误中断（didStopWithError）时返回 1；上层把它当录音失败处理。 */
int echo_system_audio_stream_failed(void);

#ifdef __cplusplus
}
#endif

#endif
