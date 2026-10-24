#import "recording_bridge.h"

#if defined(__APPLE__)
#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>

static AVAudioRecorder *echo_recorder = nil;
static NSObject *echo_recording_lock = nil;

static NSObject *recording_lock(void) {
    static dispatch_once_t once_token;
    dispatch_once(&once_token, ^{
        echo_recording_lock = [NSObject new];
    });
    return echo_recording_lock;
}

static int permission_status(void) {
    if (@available(macOS 10.14, *)) {
        switch ([AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio]) {
            case AVAuthorizationStatusNotDetermined:
                return ECHO_RECORDING_PERMISSION_UNDETERMINED;
            case AVAuthorizationStatusDenied:
            case AVAuthorizationStatusRestricted:
                return ECHO_RECORDING_PERMISSION_DENIED_STATUS;
            case AVAuthorizationStatusAuthorized:
                return ECHO_RECORDING_PERMISSION_GRANTED;
        }
    }
    return ECHO_RECORDING_PERMISSION_UNSUPPORTED_STATUS;
}

int echo_recording_permission_status(void) {
    return permission_status();
}

int echo_recording_request_permission(void) {
    if (@available(macOS 10.14, *)) {
        int current = permission_status();
        if (current == ECHO_RECORDING_PERMISSION_GRANTED) {
            return 1;
        }
        if (current == ECHO_RECORDING_PERMISSION_DENIED_STATUS) {
            return 0;
        }

        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block BOOL granted = NO;
        // 授权框由主线程呈现：在后台队列调用 requestAccessForMediaType 时，
        // TCC 会既不弹窗、也不回调，调用方就永远等下去（「准备中…」卡死即由此而来）。
        dispatch_async(dispatch_get_main_queue(), ^{
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                                     completionHandler:^(BOOL access_granted) {
                granted = access_granted;
                dispatch_semaphore_signal(semaphore);
            }];
        });
        // 有界等待：超时返回 -2，让上层给出「去系统设置手动允许」这种可操作的错误，
        // 而不是无限挂起。
        if (dispatch_semaphore_wait(semaphore,
                                    dispatch_time(DISPATCH_TIME_NOW, (int64_t)(120 * NSEC_PER_SEC))) != 0) {
            return -2;
        }
        return granted ? 1 : 0;
    }
    return -1;
}

int echo_recording_start(const char *path) {
    if (path == NULL || path[0] == '\0') {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }

    int permission = permission_status();
    if (permission == ECHO_RECORDING_PERMISSION_UNDETERMINED) {
        int requested = echo_recording_request_permission();
        if (requested == -2) {
            return ECHO_RECORDING_PERMISSION_TIMEOUT;
        }
        if (requested < 0) {
            return ECHO_RECORDING_UNSUPPORTED;
        }
        if (requested == 0) {
            return ECHO_RECORDING_PERMISSION_DENIED;
        }
    } else if (permission == ECHO_RECORDING_PERMISSION_DENIED_STATUS) {
        return ECHO_RECORDING_PERMISSION_DENIED;
    } else if (permission == ECHO_RECORDING_PERMISSION_UNSUPPORTED_STATUS) {
        return ECHO_RECORDING_UNSUPPORTED;
    }

    @synchronized(recording_lock()) {
        if (echo_recorder != nil) {
            return ECHO_RECORDING_BUSY;
        }

        NSString *path_string = [[NSString alloc] initWithUTF8String:path];
        if (path_string == nil) {
            return ECHO_RECORDING_INVALID_ARGUMENT;
        }
        NSURL *file_url = [NSURL fileURLWithPath:path_string];
        NSDictionary *settings = @{
            AVFormatIDKey: @(kAudioFormatLinearPCM),
            AVSampleRateKey: @48000.0,
            AVNumberOfChannelsKey: @1,
            AVLinearPCMBitDepthKey: @16,
            AVLinearPCMIsBigEndianKey: @NO,
            AVLinearPCMIsFloatKey: @NO,
        };

        NSError *error = nil;
        AVAudioRecorder *recorder = [[AVAudioRecorder alloc] initWithURL:file_url
                                                                settings:settings
                                                                   error:&error];
        if (recorder == nil || error != nil || ![recorder prepareToRecord]) {
            return ECHO_RECORDING_AUDIO_ERROR;
        }
        recorder.meteringEnabled = YES;
        if (![recorder record]) {
            return ECHO_RECORDING_AUDIO_ERROR;
        }
        echo_recorder = recorder;
        return ECHO_RECORDING_OK;
    }
}

int echo_recording_pause(void) {
    @synchronized(recording_lock()) {
        if (echo_recorder == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        [echo_recorder pause];
        return ECHO_RECORDING_OK;
    }
}

int echo_recording_resume(void) {
    @synchronized(recording_lock()) {
        if (echo_recorder == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        return [echo_recorder record] ? ECHO_RECORDING_OK : ECHO_RECORDING_AUDIO_ERROR;
    }
}

int echo_recording_stop(void) {
    @synchronized(recording_lock()) {
        if (echo_recorder == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        [echo_recorder stop];
        echo_recorder = nil;
        return ECHO_RECORDING_OK;
    }
}

int echo_recording_cancel(void) {
    @synchronized(recording_lock()) {
        if (echo_recorder == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        [echo_recorder stop];
        BOOL deleted = [echo_recorder deleteRecording];
        echo_recorder = nil;
        return deleted ? ECHO_RECORDING_OK : ECHO_RECORDING_AUDIO_ERROR;
    }
}

int echo_recording_is_recording(void) {
    @synchronized(recording_lock()) {
        return echo_recorder != nil && echo_recorder.isRecording ? 1 : 0;
    }
}

int echo_recording_levels(float *average_db, float *peak_db) {
    if (average_db == NULL || peak_db == NULL) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }
    @synchronized(recording_lock()) {
        if (echo_recorder == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        [echo_recorder updateMeters];
        *average_db = [echo_recorder averagePowerForChannel:0];
        *peak_db = [echo_recorder peakPowerForChannel:0];
        return ECHO_RECORDING_OK;
    }
}

#else

int echo_recording_permission_status(void) { return ECHO_RECORDING_PERMISSION_UNSUPPORTED_STATUS; }
int echo_recording_request_permission(void) { return -1; }
int echo_recording_start(const char *path) { (void)path; return ECHO_RECORDING_UNSUPPORTED; }
int echo_recording_pause(void) { return ECHO_RECORDING_UNSUPPORTED; }
int echo_recording_resume(void) { return ECHO_RECORDING_UNSUPPORTED; }
int echo_recording_stop(void) { return ECHO_RECORDING_UNSUPPORTED; }
int echo_recording_cancel(void) { return ECHO_RECORDING_UNSUPPORTED; }
int echo_recording_is_recording(void) { return 0; }
int echo_recording_levels(float *average_db, float *peak_db) {
    (void)average_db;
    (void)peak_db;
    return ECHO_RECORDING_UNSUPPORTED;
}

#endif
