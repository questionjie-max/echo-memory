#ifndef ECHO_MEMORY_RECORDING_BRIDGE_H
#define ECHO_MEMORY_RECORDING_BRIDGE_H

#ifdef __cplusplus
extern "C" {
#endif

enum EchoRecordingResult {
    ECHO_RECORDING_OK = 0,
    ECHO_RECORDING_INVALID_ARGUMENT = -1,
    ECHO_RECORDING_PERMISSION_DENIED = -2,
    ECHO_RECORDING_UNSUPPORTED = -3,
    ECHO_RECORDING_BUSY = -4,
    ECHO_RECORDING_NOT_ACTIVE = -5,
    ECHO_RECORDING_AUDIO_ERROR = -6,
    /* 授权框未在限时内得到结果：上层必须把它当错误报给用户，不能无限等待。 */
    ECHO_RECORDING_PERMISSION_TIMEOUT = -7,
};

enum EchoRecordingPermission {
    ECHO_RECORDING_PERMISSION_UNDETERMINED = 0,
    ECHO_RECORDING_PERMISSION_DENIED_STATUS = 1,
    ECHO_RECORDING_PERMISSION_GRANTED = 2,
    ECHO_RECORDING_PERMISSION_UNSUPPORTED_STATUS = 3,
};

int echo_recording_permission_status(void);

/* 必须在非主线程调用（内部会到主队列发起授权请求，再阻塞当前线程等待结果）。
 * 返回 1 已授权、0 已拒绝、-1 平台不支持、-2 等待超时。 */
int echo_recording_request_permission(void);

int echo_recording_start(const char *path);
int echo_recording_pause(void);
int echo_recording_resume(void);
int echo_recording_stop(void);
int echo_recording_cancel(void);
int echo_recording_is_recording(void);
int echo_recording_levels(float *average_db, float *peak_db);

#ifdef __cplusplus
}
#endif

#endif
