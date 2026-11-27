#import "NveAppleVideoPlayerView.h"

#if !TARGET_OS_WATCH

#import <CoreMedia/CoreMedia.h>

#include <algorithm>
#include <cctype>
#include <cmath>
#include <limits>
#include <string>

namespace nve::video::apple {

namespace {

std::string normalize_url_for_scheme_check(std::string_view url) {
  std::string norm;
  norm.reserve(url.size());
  for (char ch : url) {
    const unsigned char u = static_cast<unsigned char>(ch);
    if (u <= 0x20u || u == 0x7Fu) continue;
    norm.push_back(static_cast<char>(std::tolower(u)));
  }
  return norm;
}

NSString * _Nonnull nsstring_from_sv(std::string_view sv) {
  if (sv.empty()) return @"";
  NSString *str = [[NSString alloc] initWithBytes:sv.data()
                                           length:sv.size()
                                         encoding:NSUTF8StringEncoding];
  return str ?: @"";
}

} // namespace

bool is_safe_apple_media_uri(std::string_view uri) noexcept {
  const std::string norm = normalize_url_for_scheme_check(uri);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm.starts_with("file://");
}

} // namespace nve::video::apple

NS_ASSUME_NONNULL_BEGIN

@interface NveAppleVideoPlayerView ()
@property (nonatomic, strong, readwrite) AVPlayer *nvePlayer;
@property (nonatomic, strong, readwrite) AVPlayerLayer *nvePlayerLayer;
@property (nonatomic, readwrite, copy, nullable) NSString *nveSourceUri;
@property (nonatomic, readwrite) BOOL nveAutoplay;
@property (nonatomic, readwrite) BOOL nveMuted;
@property (nonatomic, readwrite) BOOL nveLooping;
@property (nonatomic, readwrite) BOOL nveControls;
@property (nonatomic, readwrite) BOOL nveIsPlaying;
@property (nonatomic, readwrite) uint64_t nvePositionMs;
@property (nonatomic, readwrite) float nveVolume;
@property (nonatomic, readwrite) NSUInteger nvePlayCount;
@property (nonatomic, readwrite) NSUInteger nvePauseCount;
@property (nonatomic, readwrite) NSUInteger nveSeekCount;
@property (nonatomic, readwrite) NSUInteger nveVolumeChangeCount;
@end

@implementation NveAppleVideoPlayerView

#if !TARGET_OS_OSX
+ (Class)layerClass {
  return [AVPlayerLayer class];
}
#endif

- (instancetype)initWithFrame:(CGRect)frame {
  self = [super initWithFrame:frame];
  if (self) {
    _nvePlayer = [[AVPlayer alloc] init];
    _nvePlayer.actionAtItemEnd = AVPlayerActionAtItemEndPause;
#if TARGET_OS_OSX
    _nvePlayerLayer = [AVPlayerLayer playerLayerWithPlayer:_nvePlayer];
    _nvePlayerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.layer = _nvePlayerLayer;
    self.wantsLayer = YES;
    _nvePlayerLayer.frame = self.bounds;
#else
    _nvePlayerLayer = (AVPlayerLayer *)self.layer;
    _nvePlayerLayer.player = _nvePlayer;
    _nvePlayerLayer.videoGravity = AVLayerVideoGravityResizeAspect;
    self.clipsToBounds = YES;
#endif
    _nveControls = YES;
    _nveVolume = 1.0f;
  }
  return self;
}

- (void)dealloc {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
}

#if TARGET_OS_OSX
- (BOOL)isFlipped {
  return YES;
}

- (void)layout {
  [super layout];
  _nvePlayerLayer.frame = self.bounds;
}
#endif

- (void)_handleItemDidReachEnd:(NSNotification *)note {
  if (!_nveLooping || note.object != _nvePlayer.currentItem) return;
  [_nvePlayer seekToTime:kCMTimeZero];
  [_nvePlayer play];
}

- (void)applyVideoProps:(const nve::video::apple::VideoPlayerProps &)props {
  NSString *nextUri = nve::video::apple::nsstring_from_sv(props.source_uri);
  const BOOL uriChanged = (_nveSourceUri == nil || ![_nveSourceUri isEqualToString:nextUri]);
  if (uriChanged && nve::video::apple::is_safe_apple_media_uri(props.source_uri)) {
    NSURL *url = [NSURL URLWithString:nextUri];
    if (url != nil) {
      [[NSNotificationCenter defaultCenter] removeObserver:self
                                                      name:AVPlayerItemDidPlayToEndTimeNotification
                                                    object:_nvePlayer.currentItem];
      _nveSourceUri = [nextUri copy];
      AVPlayerItem *item = [AVPlayerItem playerItemWithURL:url];
      [_nvePlayer replaceCurrentItemWithPlayerItem:item];
      [[NSNotificationCenter defaultCenter] addObserver:self
                                               selector:@selector(_handleItemDidReachEnd:)
                                                   name:AVPlayerItemDidPlayToEndTimeNotification
                                                 object:item];
    }
  }
  _nveAutoplay = props.autoplay ? YES : NO;
  _nveMuted = props.muted ? YES : NO;
  _nveLooping = props.looping ? YES : NO;
  _nveControls = props.controls ? YES : NO;
  _nvePlayer.muted = _nveMuted;
  _nvePlayer.actionAtItemEnd =
      _nveLooping ? AVPlayerActionAtItemEndNone : AVPlayerActionAtItemEndPause;
  if (_nveAutoplay && _nveSourceUri.length > 0 && !_nveIsPlaying) {
    _nveIsPlaying = YES;
    ++_nvePlayCount;
    [_nvePlayer play];
  }
}

- (BOOL)dispatchVideoCommand:(const nve::video::apple::VideoPlayerCommand &)cmd {
  using Kind = nve::video::apple::VideoPlayerCommandKind;
  switch (cmd.kind) {
    case Kind::Play:
      _nveIsPlaying = YES;
      ++_nvePlayCount;
      [_nvePlayer play];
      return YES;
    case Kind::Pause:
      _nveIsPlaying = NO;
      ++_nvePauseCount;
      [_nvePlayer pause];
      return YES;
    case Kind::SeekMs: {
      _nvePositionMs = cmd.seek_ms;
      ++_nveSeekCount;
      const uint64_t capped = std::min<uint64_t>(
          cmd.seek_ms, static_cast<uint64_t>(std::numeric_limits<int64_t>::max()));
      [_nvePlayer seekToTime:CMTimeMake(static_cast<int64_t>(capped), 1000)];
      return YES;
    }
    case Kind::SetVolume: {
      if (!std::isfinite(cmd.volume)) return NO;
      _nveVolume = std::clamp(cmd.volume, 0.0f, 1.0f);
      ++_nveVolumeChangeCount;
      _nvePlayer.volume = _nveVolume;
      return YES;
    }
  }
  return NO;
}

- (void)stopAndCleanup {
  [[NSNotificationCenter defaultCenter] removeObserver:self];
  _nveIsPlaying = NO;
  [_nvePlayer pause];
  [_nvePlayer replaceCurrentItemWithPlayerItem:nil];
}

@end

NS_ASSUME_NONNULL_END
#endif
