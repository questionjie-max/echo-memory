#import "recording_bridge.h"

#if defined(__APPLE__)
#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudio/CoreAudio.h>
#import <Foundation/Foundation.h>
#import <math.h>

static AVAudioEngine *echo_engine = nil;
static ExtAudioFileRef echo_audio_file = NULL;
static NSURL *echo_output_url = nil;
static NSString *echo_device_uid = nil;
static NSObject *echo_recording_lock = nil;
static dispatch_queue_t echo_write_queue = nil;
static uint64_t echo_generation = 0;
static float echo_average_db = -100.0f;
static float echo_peak_db = -100.0f;

static NSObject *recording_lock(void) {
    static dispatch_once_t once_token;
    dispatch_once(&once_token, ^{
        echo_recording_lock = [NSObject new];
    });
    return echo_recording_lock;
}

static dispatch_queue_t write_queue(void) {
    static dispatch_once_t once_token;
    dispatch_once(&once_token, ^{
        echo_write_queue = dispatch_queue_create("com.echomemory.microphone-write", DISPATCH_QUEUE_SERIAL);
    });
    return echo_write_queue;
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
        dispatch_async(dispatch_get_main_queue(), ^{
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                                     completionHandler:^(BOOL access_granted) {
                granted = access_granted;
                dispatch_semaphore_signal(semaphore);
            }];
        });
        if (dispatch_semaphore_wait(semaphore,
                                    dispatch_time(DISPATCH_TIME_NOW, (int64_t)(120 * NSEC_PER_SEC))) != 0) {
            return -2;
        }
        return granted ? 1 : 0;
    }
    return -1;
}

static AudioDeviceID default_input_device(void) {
    AudioObjectPropertyAddress address = {
        kAudioHardwarePropertyDefaultInputDevice,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    AudioDeviceID device = kAudioObjectUnknown;
    UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) != noErr) {
        return kAudioObjectUnknown;
    }
    return device;
}

static NSString *device_string_property(AudioDeviceID device, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress address = {
        selector,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &value) != noErr || value == NULL) {
        return nil;
    }
    return (__bridge_transfer NSString *)value;
}

static BOOL device_has_input_channels(AudioDeviceID device) {
    AudioObjectPropertyAddress address = {
        kAudioDevicePropertyStreamConfiguration,
        kAudioObjectPropertyScopeInput,
        kAudioObjectPropertyElementMain,
    };
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(device, &address, 0, NULL, &size) != noErr || size == 0) {
        return NO;
    }
    AudioBufferList *buffers = (AudioBufferList *)malloc(size);
    if (buffers == NULL) {
        return NO;
    }
    BOOL has_input = NO;
    if (AudioObjectGetPropertyData(device, &address, 0, NULL, &size, buffers) == noErr) {
        for (UInt32 index = 0; index < buffers->mNumberBuffers; index++) {
            if (buffers->mBuffers[index].mNumberChannels > 0) {
                has_input = YES;
                break;
            }
        }
    }
    free(buffers);
    return has_input;
}

static NSArray<NSDictionary *> *available_input_devices(void) {
    AudioObjectPropertyAddress address = {
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    };
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &size) != noErr || size == 0) {
        return @[];
    }
    UInt32 count = size / sizeof(AudioDeviceID);
    AudioDeviceID *ids = (AudioDeviceID *)malloc(size);
    if (ids == NULL) {
        return @[];
    }
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, ids) != noErr) {
        free(ids);
        return @[];
    }
    AudioDeviceID default_device = default_input_device();
    NSMutableArray<NSDictionary *> *devices = [NSMutableArray array];
    for (UInt32 index = 0; index < count; index++) {
        AudioDeviceID device = ids[index];
        if (!device_has_input_channels(device)) {
            continue;
        }
        NSString *uid = device_string_property(device, kAudioDevicePropertyDeviceUID);
        NSString *name = device_string_property(device, kAudioDevicePropertyDeviceNameCFString);
        if (uid.length == 0 || name.length == 0) {
            continue;
        }
        [devices addObject:@{
            @"uid": uid,
            @"name": name,
            @"default": @(device == default_device),
        }];
    }
    free(ids);
    [devices sortUsingComparator:^NSComparisonResult(NSDictionary *left, NSDictionary *right) {
        BOOL left_default = [left[@"default"] boolValue];
        BOOL right_default = [right[@"default"] boolValue];
        if (left_default != right_default) {
            return left_default ? NSOrderedAscending : NSOrderedDescending;
        }
        return [left[@"name"] localizedCaseInsensitiveCompare:right[@"name"]];
    }];
    return devices;
}

int echo_recording_list_devices(echo_recording_device_callback callback, void *context) {
    if (callback == NULL) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }
    for (NSDictionary *device in available_input_devices()) {
        callback([device[@"uid"] UTF8String],
                 [device[@"name"] UTF8String],
                 [device[@"default"] boolValue] ? 1 : 0,
                 context);
    }
    return ECHO_RECORDING_OK;
}

static AudioDeviceID input_device_for_uid(NSString *uid) {
    if (uid.length == 0) {
        return default_input_device();
    }
    for (NSDictionary *device in available_input_devices()) {
        if ([device[@"uid"] isEqualToString:uid]) {
            AudioObjectPropertyAddress address = {
                kAudioHardwarePropertyDevices,
                kAudioObjectPropertyScopeGlobal,
                kAudioObjectPropertyElementMain,
            };
            UInt32 size = 0;
            if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &size) != noErr) {
                return kAudioObjectUnknown;
            }
            UInt32 count = size / sizeof(AudioDeviceID);
            AudioDeviceID *ids = (AudioDeviceID *)malloc(size);
            if (ids == NULL) {
                return kAudioObjectUnknown;
            }
            AudioDeviceID matched = kAudioObjectUnknown;
            if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, ids) == noErr) {
                for (UInt32 index = 0; index < count; index++) {
                    NSString *candidate = device_string_property(ids[index], kAudioDevicePropertyDeviceUID);
                    if ([candidate isEqualToString:uid]) {
                        matched = ids[index];
                        break;
                    }
                }
            }
            free(ids);
            return matched;
        }
    }
    return kAudioObjectUnknown;
}

static AVAudioPCMBuffer *copy_pcm_buffer(AVAudioPCMBuffer *source) {
    AVAudioPCMBuffer *copy = [[AVAudioPCMBuffer alloc] initWithPCMFormat:source.format
                                                           frameCapacity:source.frameLength];
    if (copy == nil) {
        return nil;
    }
    copy.frameLength = source.frameLength;
    const AudioBufferList *source_list = source.audioBufferList;
    const AudioBufferList *copy_list = copy.audioBufferList;
    for (UInt32 index = 0; index < source_list->mNumberBuffers && index < copy_list->mNumberBuffers; index++) {
        UInt32 bytes = MIN(source_list->mBuffers[index].mDataByteSize,
                           copy_list->mBuffers[index].mDataByteSize);
        if (bytes > 0 && source_list->mBuffers[index].mData != NULL && copy_list->mBuffers[index].mData != NULL) {
            memcpy(copy_list->mBuffers[index].mData, source_list->mBuffers[index].mData, bytes);
        }
    }
    return copy;
}

static void update_levels(AVAudioPCMBuffer *buffer) {
    if (buffer.floatChannelData == NULL || buffer.frameLength == 0 || buffer.format.channelCount == 0) {
        return;
    }
    double sum = 0.0;
    float peak = 0.0f;
    NSUInteger frame_count = buffer.frameLength;
    AVAudioChannelCount channels = buffer.format.channelCount;
    if (buffer.format.isInterleaved) {
        float *samples = buffer.floatChannelData[0];
        NSUInteger sample_count = frame_count * channels;
        for (NSUInteger index = 0; index < sample_count; index++) {
            float value = samples[index];
            sum += (double)value * value;
            peak = fmaxf(peak, fabsf(value));
        }
        sum /= (double)sample_count;
    } else {
        for (AVAudioChannelCount channel = 0; channel < channels; channel++) {
            float *samples = buffer.floatChannelData[channel];
            for (NSUInteger index = 0; index < frame_count; index++) {
                float value = samples[index];
                sum += (double)value * value;
                peak = fmaxf(peak, fabsf(value));
            }
        }
        sum /= (double)(frame_count * channels);
    }
    float rms = (float)sqrt(sum);
    echo_average_db = rms > 0.0f ? fmaxf(-100.0f, 20.0f * log10f(rms)) : -100.0f;
    echo_peak_db = peak > 0.0f ? fmaxf(-100.0f, 20.0f * log10f(peak)) : -100.0f;
}

static void reset_recording_state(void) {
    echo_engine = nil;
    echo_audio_file = NULL;
    echo_output_url = nil;
    echo_device_uid = nil;
    echo_average_db = -100.0f;
    echo_peak_db = -100.0f;
}

int echo_recording_start(const char *path, const char *device_uid) {
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

    NSString *path_string = [[NSString alloc] initWithUTF8String:path];
    if (path_string == nil) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }
    NSString *selected_uid = device_uid == NULL ? @"" : [NSString stringWithUTF8String:device_uid];
    if (selected_uid == nil) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }
    AudioDeviceID device = input_device_for_uid(selected_uid);
    if (device == kAudioObjectUnknown) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }

    @synchronized(recording_lock()) {
        if (echo_engine != nil || echo_audio_file != NULL) {
            return ECHO_RECORDING_BUSY;
        }

        NSURL *url = [NSURL fileURLWithPath:path_string];
        AudioStreamBasicDescription file_format = {
            .mSampleRate = 48000.0,
            .mFormatID = kAudioFormatLinearPCM,
            .mFormatFlags = kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
            .mBytesPerPacket = 2,
            .mFramesPerPacket = 1,
            .mBytesPerFrame = 2,
            .mChannelsPerFrame = 1,
            .mBitsPerChannel = 16,
        };
        ExtAudioFileRef file = NULL;
        OSStatus status = ExtAudioFileCreateWithURL((__bridge CFURLRef)url,
                                                    kAudioFileWAVEType,
                                                    &file_format,
                                                    NULL,
                                                    kAudioFileFlags_EraseFile,
                                                    &file);
        if (status != noErr || file == NULL) {
            return ECHO_RECORDING_AUDIO_ERROR;
        }

        // 引擎安装与启动整段包 @try/@catch：AVAudioEngine 系列 API 在设备
        // 格式异常等边界条件下会抛 NSException，异常穿过 C ABI 进入 Rust 时
        // 无法被 catch_unwind 兜住（Rust 只挡得住 Rust panic），进程直接
        // abort——2026-11-11 实机验证过这条路径会整体闪退。兜底后任何原生
        // 异常都降级为 AUDIO_ERROR，应用存活并给出可行动错误。
        AVAudioEngine *engine = nil;
        @try {
        engine = [[AVAudioEngine alloc] init];
        AVAudioInputNode *input = engine.inputNode;
        AudioUnit audio_unit = input.audioUnit;
        status = AudioUnitSetProperty(audio_unit,
                                      kAudioOutputUnitProperty_CurrentDevice,
                                      kAudioUnitScope_Global,
                                      0,
                                      &device,
                                      sizeof(device));
        if (status != noErr) {
            ExtAudioFileDispose(file);
            return ECHO_RECORDING_AUDIO_ERROR;
        }

        // 输入节点必须用 inputFormatForBus:0 取硬件格式。
        // outputFormatForBus:0 返回的是引擎「期望交付」的格式（默认设备、
        // 切换前的陈旧值），与切过来的设备真实格式不一致时，
        // installTapOnBus 会抛 NSException（"Input HW format and tap format
        // not matching" / "format.sampleRate == inputHWFormat.sampleRate"），
        // 且该异常穿过 C ABI 会直接 abort 整个进程。2026-11-11 实机复现：
        // 切换设备后 outputFormatForBus 报 44100Hz，硬件实为 48000Hz。
        AVAudioFormat *format = [input inputFormatForBus:0];
        if (format == nil || format.sampleRate <= 0 || format.channelCount == 0) {
            ExtAudioFileDispose(file);
            return ECHO_RECORDING_AUDIO_ERROR;
        }
        AudioStreamBasicDescription client_format = *format.streamDescription;
        status = ExtAudioFileSetProperty(file,
                                         kExtAudioFileProperty_ClientDataFormat,
                                         sizeof(client_format),
                                         &client_format);
        if (status != noErr) {
            ExtAudioFileDispose(file);
            return ECHO_RECORDING_AUDIO_ERROR;
        }

        echo_generation += 1;
        uint64_t generation = echo_generation;
        echo_audio_file = file;
        echo_output_url = url;
        echo_device_uid = [selected_uid copy];
        echo_average_db = -100.0f;
        echo_peak_db = -100.0f;

        [input installTapOnBus:0
                   bufferSize:4096
                       format:format
                        block:^(AVAudioPCMBuffer *buffer, __unused AVAudioTime *when) {
            AVAudioPCMBuffer *copy = copy_pcm_buffer(buffer);
            if (copy == nil) {
                return;
            }
            dispatch_async(write_queue(), ^{
                @synchronized(recording_lock()) {
                    if (generation != echo_generation || echo_audio_file == NULL) {
                        return;
                    }
                    ExtAudioFileWrite(echo_audio_file, copy.frameLength, copy.audioBufferList);
                    update_levels(copy);
                }
            });
        }];

        NSError *error = nil;
        if (![engine startAndReturnError:&error]) {
            [input removeTapOnBus:0];
            ExtAudioFileDispose(file);
            reset_recording_state();
            return ECHO_RECORDING_AUDIO_ERROR;
        }
        echo_engine = engine;
        return ECHO_RECORDING_OK;
        } @catch (NSException *exception) {
            // 原生异常兜底：清理半初始化的引擎/文件，返回错误码而不是让进程 abort。
            NSLog(@"[echo-memory] 录音启动被原生异常中断：%@：%@",
                  exception.name, exception.reason);
            if (engine != nil) {
                @try {
                    [engine.inputNode removeTapOnBus:0];
                } @catch (NSException *ignored) {
                }
            }
            // 异常可能发生在 echo_audio_file 赋值前或后，两个引用都要收尾。
            ExtAudioFileRef open_file = echo_audio_file != NULL ? echo_audio_file : file;
            if (open_file != NULL) {
                ExtAudioFileDispose(open_file);
            }
            reset_recording_state();
            return ECHO_RECORDING_AUDIO_ERROR;
        }
    }
}

static int stop_recording(BOOL delete_file) {
    AVAudioEngine *engine = nil;
    ExtAudioFileRef file = NULL;
    NSURL *url = nil;
    @synchronized(recording_lock()) {
        if (echo_engine == nil && echo_audio_file == NULL) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        engine = echo_engine;
        file = echo_audio_file;
        url = echo_output_url;
        echo_engine = nil;
    }

    if (engine != nil) {
        [engine.inputNode removeTapOnBus:0];
        [engine stop];
    }
    dispatch_sync(write_queue(), ^{
        // 等已排队的 PCM 写完，再关闭文件头。
    });

    @synchronized(recording_lock()) {
        echo_generation += 1;
        echo_audio_file = NULL;
        echo_output_url = nil;
        echo_device_uid = nil;
        echo_average_db = -100.0f;
        echo_peak_db = -100.0f;
    }

    OSStatus status = file == NULL ? noErr : ExtAudioFileDispose(file);
    if (delete_file && url != nil) {
        [[NSFileManager defaultManager] removeItemAtURL:url error:nil];
    }
    return status == noErr ? ECHO_RECORDING_OK : ECHO_RECORDING_AUDIO_ERROR;
}

int echo_recording_pause(void) {
    @synchronized(recording_lock()) {
        if (echo_engine == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        [echo_engine pause];
        return ECHO_RECORDING_OK;
    }
}

int echo_recording_resume(void) {
    @synchronized(recording_lock()) {
        if (echo_engine == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        NSError *error = nil;
        return [echo_engine startAndReturnError:&error] ? ECHO_RECORDING_OK : ECHO_RECORDING_AUDIO_ERROR;
    }
}

int echo_recording_stop(void) {
    return stop_recording(NO);
}

int echo_recording_cancel(void) {
    return stop_recording(YES);
}

int echo_recording_is_recording(void) {
    @synchronized(recording_lock()) {
        return echo_engine != nil ? 1 : 0;
    }
}

int echo_recording_levels(float *average_db, float *peak_db) {
    if (average_db == NULL || peak_db == NULL) {
        return ECHO_RECORDING_INVALID_ARGUMENT;
    }
    @synchronized(recording_lock()) {
        if (echo_engine == nil) {
            return ECHO_RECORDING_NOT_ACTIVE;
        }
        *average_db = echo_average_db;
        *peak_db = echo_peak_db;
        return ECHO_RECORDING_OK;
    }
}

#else

int echo_recording_permission_status(void) { return ECHO_RECORDING_PERMISSION_UNSUPPORTED_STATUS; }
int echo_recording_request_permission(void) { return -1; }
int echo_recording_list_devices(echo_recording_device_callback callback, void *context) {
    (void)callback;
    (void)context;
    return ECHO_RECORDING_UNSUPPORTED;
}
int echo_recording_start(const char *path, const char *device_uid) {
    (void)path;
    (void)device_uid;
    return ECHO_RECORDING_UNSUPPORTED;
}
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
