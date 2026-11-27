#pragma once

#import <Foundation/Foundation.h>
#import <QuartzCore/QuartzCore.h>
#import <TargetConditionals.h>

#if !TARGET_OS_WATCH
#import <AVFoundation/AVFoundation.h>

#if TARGET_OS_OSX
#import <AppKit/AppKit.h>
typedef NSView NveVideoPlatformView;
#else
#import <UIKit/UIKit.h>
typedef UIView NveVideoPlatformView;
#endif

#include <cstdint>
#include <string_view>

namespace nve::video::apple {

struct VideoPlayerProps {
  std::string_view source_uri;
  bool autoplay{false};
  bool muted{false};
  bool looping{false};
  bool controls{true};
};

enum class VideoPlayerCommandKind : uint32_t {
  Play = 1,
  Pause = 2,
  SeekMs = 3,
  SetVolume = 4,
};

struct VideoPlayerCommand {
  VideoPlayerCommandKind kind{VideoPlayerCommandKind::Play};
  uint64_t seek_ms{0};
  float volume{1.0f};
};

/// Returns `true` if `uri` uses a safe media scheme (`https://`, `http://`, or `file://`).
bool is_safe_apple_media_uri(std::string_view uri) noexcept;

} // namespace nve::video::apple

NS_ASSUME_NONNULL_BEGIN

/// Shared Apple (AppKit & UIKit) native `AVPlayerLayer` adapter for `nve.video_player`.
@interface NveAppleVideoPlayerView : NveVideoPlatformView
@property (nonatomic, strong, readonly) AVPlayer *nvePlayer;
@property (nonatomic, strong, readonly) AVPlayerLayer *nvePlayerLayer;
@property (nonatomic, readonly, copy, nullable) NSString *nveSourceUri;
@property (nonatomic, readonly) BOOL nveAutoplay;
@property (nonatomic, readonly) BOOL nveMuted;
@property (nonatomic, readonly) BOOL nveLooping;
@property (nonatomic, readonly) BOOL nveControls;
@property (nonatomic, readonly) BOOL nveIsPlaying;
@property (nonatomic, readonly) uint64_t nvePositionMs;
@property (nonatomic, readonly) float nveVolume;
@property (nonatomic, readonly) NSUInteger nvePlayCount;
@property (nonatomic, readonly) NSUInteger nvePauseCount;
@property (nonatomic, readonly) NSUInteger nveSeekCount;
@property (nonatomic, readonly) NSUInteger nveVolumeChangeCount;

- (void)applyVideoProps:(const nve::video::apple::VideoPlayerProps &)props;
- (BOOL)dispatchVideoCommand:(const nve::video::apple::VideoPlayerCommand &)cmd;
- (void)stopAndCleanup;
@end

NS_ASSUME_NONNULL_END
#endif
