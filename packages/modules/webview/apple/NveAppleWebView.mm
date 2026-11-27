#import "NveAppleWebView.h"

#if !TARGET_OS_TV && !TARGET_OS_WATCH

#include <cctype>
#include <string>

namespace nve::webview::apple {

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

bool is_safe_apple_webview_url(std::string_view url) noexcept {
  const std::string norm = normalize_url_for_scheme_check(url);
  if (norm.empty()) return false;
  return norm.starts_with("https://") || norm.starts_with("http://") || norm == "about:blank";
}

} // namespace nve::webview::apple

NS_ASSUME_NONNULL_BEGIN

@interface NveAppleWebView ()
@property (nonatomic, readwrite, copy, nullable) NSString *nveLastRequestedUrl;
@property (nonatomic, readwrite) BOOL nveAllowsBackForward;
@property (nonatomic, readwrite) NSUInteger nveNavigationCount;
@property (nonatomic, readwrite) NSUInteger nveReloadCount;
@property (nonatomic, readwrite) NSUInteger nveGoBackCount;
@property (nonatomic, readwrite) NSUInteger nveGoForwardCount;
@property (nonatomic, readwrite, copy, nullable) NSString *nveLastEvaluatedScript;
@end

@implementation NveAppleWebView

+ (instancetype)makeDefaultWithFrame:(CGRect)frame {
  WKWebViewConfiguration *cfg = [[WKWebViewConfiguration alloc] init];
  return [[self alloc] initWithFrame:frame configuration:cfg];
}

- (instancetype)initWithFrame:(CGRect)frame configuration:(WKWebViewConfiguration *)configuration {
  self = [super initWithFrame:frame configuration:configuration];
  if (self) {
    _nveAllowsBackForward = YES;
    self.allowsBackForwardNavigationGestures = YES;
  }
  return self;
}

- (BOOL)_loadSafeUrl:(std::string_view)url {
  if (!nve::webview::apple::is_safe_apple_webview_url(url)) {
    return NO;
  }
  NSString *nsStr = nve::webview::apple::nsstring_from_sv(url);
  NSURL *nsUrl = [NSURL URLWithString:nsStr];
  if (nsUrl == nil) return NO;
  _nveLastRequestedUrl = [nsStr copy];
  ++_nveNavigationCount;
  [self loadRequest:[NSURLRequest requestWithURL:nsUrl]];
  return YES;
}

- (void)applyWebViewProps:(const nve::webview::apple::WebViewProps &)props {
  _nveAllowsBackForward = props.allows_back_forward ? YES : NO;
  self.allowsBackForwardNavigationGestures = _nveAllowsBackForward;
  NSString *nextUrl = nve::webview::apple::nsstring_from_sv(props.url);
  if (_nveLastRequestedUrl == nil || ![_nveLastRequestedUrl isEqualToString:nextUrl]) {
    (void)[self _loadSafeUrl:props.url];
  }
}

- (BOOL)dispatchWebViewCommand:(const nve::webview::apple::WebViewCommand &)cmd {
  using Kind = nve::webview::apple::WebViewCommandKind;
  switch (cmd.kind) {
    case Kind::Navigate:
      return [self _loadSafeUrl:cmd.text];
    case Kind::Reload:
      ++_nveReloadCount;
      [self reload];
      return YES;
    case Kind::GoBack:
      if (!_nveAllowsBackForward) return NO;
      ++_nveGoBackCount;
      if (self.canGoBack) [self goBack];
      return YES;
    case Kind::GoForward:
      if (!_nveAllowsBackForward) return NO;
      ++_nveGoForwardCount;
      if (self.canGoForward) [self goForward];
      return YES;
    case Kind::EvaluateScript: {
      NSString *js = nve::webview::apple::nsstring_from_sv(cmd.text);
      _nveLastEvaluatedScript = [js copy];
      [self evaluateJavaScript:js completionHandler:nil];
      return YES;
    }
  }
  return NO;
}

@end

NS_ASSUME_NONNULL_END
#endif
