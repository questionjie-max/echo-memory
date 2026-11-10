#import "system_audio_bridge.h"

#if defined(__APPLE__)
#import <CoreGraphics/CoreGraphics.h>
#import <CoreMedia/CoreMedia.h>
#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

@class EchoSystemAudioOutput;

static NSObject *system_audio_lock = nil;
static SCStream *system_audio_stream = nil;
/* SCStream 不保证强持有 addStreamOutput 注册的 output 对象（实测弱引用时回调
 * 一个都不会来，流空转），必须自己持有强引用，stop 时释放。 */
static EchoSystemAudioOutput *system_audio_output = nil;
static BOOL system_audio_stream_failed = NO;
static echo_system_audio_sample_callback system_audio_pending_callback = nil;

static NSObject *system_audio_lock_object(void) {
    static dispatch_once_t once_token;
    dispatch_once(&once_token, ^{
        system_audio_lock = [NSObject new];
    });
    return system_audio_lock;
}

static dispatch_queue_t system_audio_queue(void) {
    static dispatch_once_t once_token;
    static dispatch_queue_t queue = nil;
    dispatch_once(&once_token, ^{
        queue = dispatch_queue_create("com.echomemory.system-audio", DISPATCH_QUEUE_SERIAL);
    });
    return queue;
}

/* ---- 授权：屏幕（含系统声音）录制 ----
 * macOS 15 起 TCC 里的开关叫「屏幕与系统声音录制」，与屏幕录制是同一个授权项，
 * 所以这里用 CoreGraphics 的屏幕捕获预检 API 判定，不再区分系统版本分支。 */
static int system_audio_permission_status(void) {
    if (CGPreflightScreenCaptureAccess()) {
        return ECHO_SYSTEM_AUDIO_PERMISSION_GRANTED;
    }
    return ECHO_SYSTEM_AUDIO_PERMISSION_DENIED_STATUS;
}

int echo_system_audio_permission_status(void) {
    return system_audio_permission_status();
}

int echo_system_audio_request_permission(void) {
    if (system_audio_permission_status() == ECHO_SYSTEM_AUDIO_PERMISSION_GRANTED) {
        return 1;
    }
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    dispatch_async(dispatch_get_main_queue(), ^{
        CGRequestScreenCaptureAccess();
        dispatch_semaphore_signal(semaphore);
    });
    if (dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(120 * NSEC_PER_SEC))) != 0) {
        return -2;
    }
    // 授权框被用户关掉后 TCC 才落库，回调返回时预检可能仍是否——给一个有界轮询窗口。
    for (int attempt = 0; attempt < 20; attempt++) {
        if (CGPreflightScreenCaptureAccess()) {
            return 1;
        }
        [NSThread sleepForTimeInterval:0.5];
    }
    return CGPreflightScreenCaptureAccess() ? 1 : 0;
}

static int fetch_shareable_content(SCShareableContent **out_content) {
    __block SCShareableContent *content = nil;
    __block NSError *content_error = nil;
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                               onScreenWindowsOnly:YES
                                                 completionHandler:^(SCShareableContent *shareable, NSError *error) {
        content = shareable;
        content_error = error;
        dispatch_semaphore_signal(semaphore);
    }];
    if (dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(20 * NSEC_PER_SEC))) != 0) {
        return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
    }
    if (content_error != nil || content == nil) {
        return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
    }
    *out_content = content;
    return ECHO_SYSTEM_AUDIO_OK;
}

int echo_system_audio_list_applications(echo_system_audio_application_callback callback,
                                        void *context) {
    if (callback == NULL) {
        return ECHO_SYSTEM_AUDIO_INVALID_ARGUMENT;
    }
    if (@available(macOS 12.3, *)) {
        if (system_audio_permission_status() != ECHO_SYSTEM_AUDIO_PERMISSION_GRANTED) {
            return ECHO_SYSTEM_AUDIO_PERMISSION_DENIED;
        }
        SCShareableContent *content = nil;
        int result = fetch_shareable_content(&content);
        if (result != ECHO_SYSTEM_AUDIO_OK) {
            return result;
        }
        NSString *own_bundle = [[NSBundle mainBundle] bundleIdentifier];
        NSMutableDictionary<NSString *, NSNumber *> *process_ids = [NSMutableDictionary dictionary];
        NSMutableDictionary<NSString *, NSString *> *names = [NSMutableDictionary dictionary];
        for (SCRunningApplication *application in content.applications) {
            NSString *bundle_id = application.bundleIdentifier;
            NSString *name = application.applicationName;
            if (bundle_id.length == 0 || name.length == 0) {
                continue;
            }
            if (own_bundle.length > 0 && [bundle_id isEqualToString:own_bundle]) {
                continue;
            }
            if (names[bundle_id] == nil) {
                names[bundle_id] = name;
                process_ids[bundle_id] = @(application.processID);
            }
        }
        NSArray<NSString *> *sorted_bundles = [names.allKeys
            sortedArrayUsingComparator:^NSComparisonResult(NSString *left, NSString *right) {
                return [names[left] localizedCaseInsensitiveCompare:names[right]];
            }];
        for (NSString *bundle_id in sorted_bundles) {
            callback(bundle_id.UTF8String,
                     names[bundle_id].UTF8String,
                     process_ids[bundle_id].intValue,
                     context);
        }
        return ECHO_SYSTEM_AUDIO_OK;
    }
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}

/* ---- 采集 ---- */

@interface EchoSystemAudioOutput : NSObject <SCStreamOutput, SCStreamDelegate>
@property (nonatomic, assign, readonly) echo_system_audio_sample_callback sampleCallback;
- (instancetype)initWithCallback:(echo_system_audio_sample_callback)callback;
@end

@implementation EchoSystemAudioOutput {
    echo_system_audio_sample_callback _callback;
}

- (instancetype)initWithCallback:(echo_system_audio_sample_callback)callback {
    self = [super init];
    if (self) {
        _callback = callback;
    }
    return self;
}

/* 把 AudioBufferList（Float32，交错或非交错都可能）摊平成交错样本后交给 Rust。 */
- (void)emitAudioBufferList:(const AudioBufferList *)bufferList
                 frameCount:(NSUInteger)frameCount
                 sampleRate:(double)sampleRate {
    if (_callback == NULL || frameCount == 0) {
        return;
    }
    BOOL isNonInterleaved = NO;
    NSUInteger channels = 1;
    const AudioBufferList *list = bufferList;
    if (list->mNumberBuffers == 1 && list->mBuffers[0].mNumberChannels > 1) {
        // 单个交错缓冲区：ASBD 的 interleaved 标志此时为 0 但带多声道数的情况不存在；
        // 交错数据只可能出现在单缓冲区里，直接按交错处理。
        channels = list->mBuffers[0].mNumberChannels;
    } else if (list->mNumberBuffers > 1) {
        isNonInterleaved = YES;
        channels = list->mNumberBuffers;
    }

    // 防越界：缓冲区里实际装得下的帧数以 mDataByteSize 为准，ASBD 报的帧数
    // 与之不符时取小者（FFI 侧只给帧数不给长度，越界读发生在 C 层无法恢复）。
    NSUInteger bytesPerFrame = sizeof(float) * channels;
    if (bytesPerFrame == 0) {
        return;
    }
    if (!isNonInterleaved) {
        NSUInteger available = list->mBuffers[0].mDataByteSize / bytesPerFrame;
        NSUInteger safeFrames = MIN(frameCount, available);
        if (safeFrames == 0) {
            return;
        }
        _callback((const float *)list->mBuffers[0].mData, (int)safeFrames, (int)channels, (unsigned int)sampleRate);
        return;
    }
    NSUInteger available = list->mBuffers[0].mDataByteSize / (sizeof(float));
    for (NSUInteger channel = 0; channel < channels; channel++) {
        available = MIN(available, list->mBuffers[channel].mDataByteSize / (sizeof(float)));
    }
    frameCount = MIN(frameCount, available);
    if (frameCount == 0) {
        return;
    }

    NSUInteger total = frameCount * channels;
    float *interleaved = (float *)malloc(total * sizeof(float));
    if (interleaved == NULL) {
        return;
    }
    for (NSUInteger channel = 0; channel < channels; channel++) {
        const float *source = (const float *)list->mBuffers[channel].mData;
        for (NSUInteger frame = 0; frame < frameCount; frame++) {
            interleaved[frame * channels + channel] = source[frame];
        }
    }
    _callback(interleaved, (int)frameCount, (int)channels, (unsigned int)sampleRate);
    free(interleaved);
}

- (void)stream:(SCStream *)stream didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer ofType:(SCStreamOutputType)type {
    if (type != SCStreamOutputTypeAudio) {
        return;
    }
    if (CMSampleBufferDataIsReady(sampleBuffer) == NO) {
        return;
    }

    CMFormatDescriptionRef format = CMSampleBufferGetFormatDescription(sampleBuffer);
    if (format == NULL) {
        return;
    }
    const AudioStreamBasicDescription *asbd = CMAudioFormatDescriptionGetStreamBasicDescription(format);
    if (asbd == NULL || asbd->mSampleRate <= 0.0) {
        return;
    }

    NSUInteger frameCount = CMSampleBufferGetNumSamples(sampleBuffer);
    if (frameCount == 0) {
        return;
    }

    // 两段式调用：先拿 AudioBufferList 需要的大小，再实际取出。
    OSStatus status = noErr;
    CMBlockBufferRef blockBuffer = NULL;
    AudioBufferList *list = NULL;
    size_t listSize = 0;
    status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sampleBuffer, &listSize, NULL, 0, kCFAllocatorDefault, NULL,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, &blockBuffer);
    if (status != noErr || listSize == 0) {
        if (blockBuffer != NULL) CFRelease(blockBuffer);
        return;
    }
    list = (AudioBufferList *)malloc(listSize);
    if (list == NULL) {
        if (blockBuffer != NULL) CFRelease(blockBuffer);
        return;
    }
    status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sampleBuffer, NULL, list, listSize, kCFAllocatorDefault, NULL,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, &blockBuffer);
    if (status == noErr) {
        [self emitAudioBufferList:list frameCount:frameCount sampleRate:asbd->mSampleRate];
    }
    free(list);
    if (blockBuffer != NULL) {
        CFRelease(blockBuffer);
    }
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    if (error != nil) {
        @synchronized(system_audio_lock_object()) {
            system_audio_stream_failed = YES;
            if (system_audio_stream == stream) {
                system_audio_stream = nil;
            }
            // 流死了 output 也要一起放掉，别把引用链滞留到下次 start。
            system_audio_output = nil;
        }
    }
}

@end

int echo_system_audio_start(const char *application_bundle_id) {
    if (@available(macOS 12.3, *)) {
        int permission = system_audio_permission_status();
        if (permission != ECHO_SYSTEM_AUDIO_PERMISSION_GRANTED) {
            int requested = echo_system_audio_request_permission();
            if (requested == -2) {
                return ECHO_SYSTEM_AUDIO_PERMISSION_TIMEOUT;
            }
            if (requested < 0) {
                return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
            }
            if (requested == 0) {
                return ECHO_SYSTEM_AUDIO_PERMISSION_DENIED;
            }
        }

        @synchronized(system_audio_lock_object()) {
            if (system_audio_stream != nil) {
                return ECHO_SYSTEM_AUDIO_BUSY;
            }
            system_audio_stream_failed = NO;

            SCShareableContent *content = nil;
            int content_result = fetch_shareable_content(&content);
            if (content_result != ECHO_SYSTEM_AUDIO_OK || content.displays.count == 0) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }
            SCDisplay *display = content.displays.firstObject;

            NSString *selected_bundle = application_bundle_id == NULL
                ? @""
                : [NSString stringWithUTF8String:application_bundle_id];
            if (selected_bundle == nil) {
                return ECHO_SYSTEM_AUDIO_INVALID_ARGUMENT;
            }
            SCContentFilter *filter = nil;
            if (selected_bundle.length > 0) {
                SCRunningApplication *selected = nil;
                for (SCRunningApplication *application in content.applications) {
                    if ([application.bundleIdentifier isEqualToString:selected_bundle]) {
                        selected = application;
                        break;
                    }
                }
                if (selected == nil) {
                    return ECHO_SYSTEM_AUDIO_INVALID_ARGUMENT;
                }
                filter = [[SCContentFilter alloc] initWithDisplay:display
                                             includingApplications:@[selected]
                                                  exceptingWindows:@[]];
            } else {
                // 排除自身进程：既从画面里去掉自己的窗口，也借 excludesCurrentProcessAudio
                // 把本应用自己发出的声音从系统声里抠掉，避免自录自的回声。
                NSMutableArray<SCRunningApplication *> *excluded = [NSMutableArray array];
                NSString *own_bundle = [[NSBundle mainBundle] bundleIdentifier];
                for (SCRunningApplication *application in content.applications) {
                    if (own_bundle.length > 0 && [application.bundleIdentifier isEqualToString:own_bundle]) {
                        [excluded addObject:application];
                    }
                }
                filter = [[SCContentFilter alloc] initWithDisplay:display
                                             excludingApplications:excluded
                                                  exceptingWindows:@[]];
            }
            if (filter == nil) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }

            SCStreamConfiguration *configuration = [[SCStreamConfiguration alloc] init];
            configuration.capturesAudio = YES;
            if (@available(macOS 13.0, *)) {
                configuration.excludesCurrentProcessAudio = YES;
            }
            configuration.sampleRate = 48000;
            configuration.channelCount = 1;
            configuration.queueDepth = 8;
            configuration.minimumFrameInterval = kCMTimeZero;
            configuration.width = (NSUInteger)(display.width / 2);
            configuration.height = (NSUInteger)(display.height / 2);
            configuration.showsCursor = NO;

            EchoSystemAudioOutput *output = [[EchoSystemAudioOutput alloc]
                initWithCallback:system_audio_pending_callback];
            SCStream *stream = [[SCStream alloc] initWithFilter:filter
                                                  configuration:configuration
                                                       delegate:output];
            if (stream == nil) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }
            NSError *output_error = nil;
            [stream addStreamOutput:output
                               type:SCStreamOutputTypeAudio
                 sampleHandlerQueue:system_audio_queue()
                              error:&output_error];
            if (output_error != nil) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }

            dispatch_semaphore_t start_semaphore = dispatch_semaphore_create(0);
            __block NSError *start_error = nil;
            [stream startCaptureWithCompletionHandler:^(NSError *capture_error) {
                start_error = capture_error;
                dispatch_semaphore_signal(start_semaphore);
            }];
            if (dispatch_semaphore_wait(start_semaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(15 * NSEC_PER_SEC))) != 0) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }
            if (start_error != nil) {
                return ECHO_SYSTEM_AUDIO_AUDIO_ERROR;
            }
            system_audio_stream = stream;
            system_audio_output = output;
            return ECHO_SYSTEM_AUDIO_OK;
        }
    }
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}

/* 停流并等待完成；文件（含 WAV 头）由 Rust 侧负责，这里只管采集本身。 */
static int system_audio_stop_stream(void) {
    SCStream *stream = nil;
    @synchronized(system_audio_lock_object()) {
        if (system_audio_stream == nil) {
            return ECHO_SYSTEM_AUDIO_NOT_ACTIVE;
        }
        stream = system_audio_stream;
        system_audio_stream = nil;
        system_audio_output = nil;
    }
    dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
    [stream stopCaptureWithCompletionHandler:^(__unused NSError *error) {
        dispatch_semaphore_signal(semaphore);
    }];
    dispatch_semaphore_wait(semaphore, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(10 * NSEC_PER_SEC)));
    return ECHO_SYSTEM_AUDIO_OK;
}

int echo_system_audio_stop(void) {
    if (@available(macOS 12.3, *)) {
        return system_audio_stop_stream();
    }
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}

int echo_system_audio_cancel(void) {
    if (@available(macOS 12.3, *)) {
        return system_audio_stop_stream();
    }
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}

int echo_system_audio_is_recording(void) {
    @synchronized(system_audio_lock_object()) {
        return system_audio_stream != nil ? 1 : 0;
    }
}

int echo_system_audio_stream_failed(void) {
    @synchronized(system_audio_lock_object()) {
        return system_audio_stream_failed ? 1 : 0;
    }
}

void echo_system_audio_set_sample_callback(echo_system_audio_sample_callback callback) {
    @synchronized(system_audio_lock_object()) {
        system_audio_pending_callback = callback;
    }
}

#else

int echo_system_audio_permission_status(void) { return ECHO_SYSTEM_AUDIO_PERMISSION_UNSUPPORTED_STATUS; }
int echo_system_audio_request_permission(void) { return -1; }
int echo_system_audio_list_applications(echo_system_audio_application_callback callback,
                                        void *context) {
    (void)callback;
    (void)context;
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}
int echo_system_audio_start(const char *application_bundle_id) {
    (void)application_bundle_id;
    return ECHO_SYSTEM_AUDIO_UNSUPPORTED;
}
int echo_system_audio_stop(void) { return ECHO_SYSTEM_AUDIO_UNSUPPORTED; }
int echo_system_audio_cancel(void) { return ECHO_SYSTEM_AUDIO_UNSUPPORTED; }
int echo_system_audio_is_recording(void) { return 0; }
int echo_system_audio_stream_failed(void) { return 0; }
void echo_system_audio_set_sample_callback(echo_system_audio_sample_callback callback) {
    (void)callback;
}

#endif
