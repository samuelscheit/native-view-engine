#pragma once

#import <Foundation/Foundation.h>
#import <TargetConditionals.h>

#if !TARGET_OS_TV && !TARGET_OS_WATCH
#import <WebKit/WebKit.h>

#include <cstdint>
#include <string>
#include <string_view>

namespace nve::webview::apple {

struct WebViewProps {
  std::string_view url;
  bool allows_back_forward{true};
};

enum class WebViewCommandKind : uint32_t {
  Navigate = 1,
  Reload = 2,
  GoBack = 3,
  GoForward = 4,
  EvaluateScript = 5,
};

struct WebViewCommand {
  WebViewCommandKind kind{WebViewCommandKind::Reload};
  std::string_view text;
};

/// Returns `true` if `url` uses a safe scheme (`https://`, `http://`, or `about:blank`).
bool is_safe_apple_webview_url(std::string_view url) noexcept;

} // namespace nve::webview::apple

NS_ASSUME_NONNULL_BEGIN

/// Shared Apple (AppKit & UIKit) native `WKWebView` adapter for `nve.web_view`.
@interface NveAppleWebView : WKWebView
@property (nonatomic, readonly, copy, nullable) NSString *nveLastRequestedUrl;
@property (nonatomic, readonly) BOOL nveAllowsBackForward;
@property (nonatomic, readonly) NSUInteger nveNavigationCount;
@property (nonatomic, readonly) NSUInteger nveReloadCount;
@property (nonatomic, readonly) NSUInteger nveGoBackCount;
@property (nonatomic, readonly) NSUInteger nveGoForwardCount;
@property (nonatomic, readonly, copy, nullable) NSString *nveLastEvaluatedScript;

+ (instancetype)makeDefaultWithFrame:(CGRect)frame;
- (void)applyWebViewProps:(const nve::webview::apple::WebViewProps &)props;
- (BOOL)dispatchWebViewCommand:(const nve::webview::apple::WebViewCommand &)cmd;
@end

NS_ASSUME_NONNULL_END
#endif
