#import <UIKit/UIKit.h>

#import "RCTNveComponentView.h"

#include <atomic>
#include <cstdint>

namespace {

std::atomic<uint32_t> gNextIosSurfaceId{1u};

uint32_t NveAllocateIosSurfaceId(void) {
  return gNextIosSurfaceId.fetch_add(1u, std::memory_order_relaxed);
}

// Dark slate background (#0f172a) matching NVE app shells.
UIColor *NveDarkSlateColor(void) {
  return [UIColor colorWithRed:(15.0 / 255.0)
                         green:(23.0 / 255.0)
                          blue:(42.0 / 255.0)
                         alpha:1.0];
}

NSString * _Nullable NveResolveAppDylibPath(void) {
  NSString *envOverride = NSProcessInfo.processInfo.environment[@"NVE_APP_DYLIB"];
  if (envOverride.length > 0) {
    return envOverride;
  }
  return [[NSBundle mainBundle] pathForResource:@"libnve_app" ofType:@"dylib"];
}

struct NveIosPairConfig {
  NSString * _Nullable endpoint;
  NSString * _Nullable token;
  NSString * _Nullable transport;
};

NveIosPairConfig NveParsePairURL(NSURL * _Nullable url) {
  NveIosPairConfig cfg{};
  if (url == nil) {
    return cfg;
  }
  NSURLComponents *comps = [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:NO];
  if (!comps || ![comps.scheme.lowercaseString isEqualToString:@"nve"] ||
      ![comps.host.lowercaseString isEqualToString:@"pair"]) {
    return cfg;
  }
  NSString *hostParam = nil;
  NSString *portParam = nil;
  for (NSURLQueryItem *item in comps.queryItems) {
    if (item.value.length == 0) {
      continue;
    }
    NSString *key = item.name.lowercaseString;
    if ([key isEqualToString:@"endpoint"] || [key isEqualToString:@"connect"] ||
        [key isEqualToString:@"url"]) {
      cfg.endpoint = item.value;
    } else if ([key isEqualToString:@"token"]) {
      cfg.token = item.value;
    } else if ([key isEqualToString:@"transport"]) {
      cfg.transport = item.value;
    } else if ([key isEqualToString:@"host"]) {
      hostParam = item.value;
    } else if ([key isEqualToString:@"port"]) {
      portParam = item.value;
    }
  }
  if (cfg.endpoint.length == 0 && hostParam.length > 0 && portParam.length > 0) {
    cfg.endpoint = [NSString stringWithFormat:@"%@:%@", hostParam, portParam];
  }
  return cfg;
}

NveIosPairConfig NveResolveLaunchPairConfig(void) {
  NveIosPairConfig cfg{};
  NSDictionary<NSString *, NSString *> *env = NSProcessInfo.processInfo.environment;
  if (env[@"NVE_PAIR_CONNECT"].length > 0) {
    cfg.endpoint = env[@"NVE_PAIR_CONNECT"];
  }
  if (env[@"NVE_PAIR_TOKEN"].length > 0) {
    cfg.token = env[@"NVE_PAIR_TOKEN"];
  }
  if (env[@"NVE_PAIR_TRANSPORT"].length > 0) {
    cfg.transport = env[@"NVE_PAIR_TRANSPORT"];
  }
  NSArray<NSString *> *args = NSProcessInfo.processInfo.arguments;
  for (NSUInteger i = 1; i < args.count; ++i) {
    NSString *flag = args[i];
    if ([flag isEqualToString:@"--pair-connect"] && i + 1 < args.count) {
      cfg.endpoint = args[++i];
    } else if ([flag isEqualToString:@"--pair-token"] && i + 1 < args.count) {
      cfg.token = args[++i];
    } else if ([flag isEqualToString:@"--pair-transport"] && i + 1 < args.count) {
      cfg.transport = args[++i];
    } else if ([flag hasPrefix:@"--pair-transport="]) {
      cfg.transport = [flag substringFromIndex:17];
    }
  }
  return cfg;
}

}  // namespace

@interface NveIosRootViewController : UIViewController
@property (nonatomic, assign) uint32_t surfaceId;
@property (nonatomic, strong) RCTNveComponentView *componentView;
@property (nonatomic, strong, nullable) UILabel *errorLabel;
@property (nonatomic, copy, nullable) NSString *pairEndpoint;
@property (nonatomic, copy, nullable) NSString *pairToken;
@property (nonatomic, copy, nullable) NSString *pairTransport;
@property (nonatomic, assign) BOOL didAttemptLoad;
- (instancetype)initWithSurfaceId:(uint32_t)surfaceId;
- (void)applyPairURL:(NSURL *)url;
- (void)disconnectPairedIfNeeded;
@end

@implementation NveIosRootViewController

- (instancetype)initWithSurfaceId:(uint32_t)surfaceId {
  self = [super initWithNibName:nil bundle:nil];
  if (self) {
    _surfaceId = surfaceId > 0u ? surfaceId : NveAllocateIosSurfaceId();
  }
  return self;
}

- (UIStatusBarStyle)preferredStatusBarStyle {
  return UIStatusBarStyleLightContent;
}

- (void)viewDidLoad {
  [super viewDidLoad];
  if (self.surfaceId == 0u) {
    self.surfaceId = NveAllocateIosSurfaceId();
  }
  self.view.backgroundColor = NveDarkSlateColor();
  if (self.pairEndpoint.length == 0) {
    const NveIosPairConfig launchPair = NveResolveLaunchPairConfig();
    self.pairEndpoint = launchPair.endpoint;
    self.pairToken = launchPair.token;
    if (self.pairTransport.length == 0) {
      self.pairTransport = launchPair.transport;
    }
  }

  RCTNveComponentView *componentView = [[RCTNveComponentView alloc] initWithFrame:CGRectZero];
  componentView.translatesAutoresizingMaskIntoConstraints = NO;
  componentView.backgroundColor = NveDarkSlateColor();
  [self.view addSubview:componentView];
  self.componentView = componentView;

  UILayoutGuide *safeArea = self.view.safeAreaLayoutGuide;
  [NSLayoutConstraint activateConstraints:@[
    [componentView.topAnchor constraintEqualToAnchor:safeArea.topAnchor],
    [componentView.leadingAnchor constraintEqualToAnchor:safeArea.leadingAnchor],
    [componentView.trailingAnchor constraintEqualToAnchor:safeArea.trailingAnchor],
    [componentView.bottomAnchor constraintEqualToAnchor:safeArea.bottomAnchor],
  ]];
}

- (void)viewDidLayoutSubviews {
  [super viewDidLayoutSubviews];
  if (!self.didAttemptLoad && CGRectGetWidth(self.componentView.bounds) > 0.0 &&
      CGRectGetHeight(self.componentView.bounds) > 0.0) {
    self.didAttemptLoad = YES;
    [self _loadBundledApp];
  }
}

- (void)viewSafeAreaInsetsDidChange {
  [super viewSafeAreaInsetsDidChange];
  [self.componentView setNeedsLayout];
}

- (void)applyPairURL:(NSURL *)url {
  const NveIosPairConfig parsed = NveParsePairURL(url);
  if (parsed.endpoint.length == 0) {
    return;
  }
  self.pairEndpoint = parsed.endpoint;
  self.pairToken = parsed.token;
  if (parsed.transport.length > 0) {
    self.pairTransport = parsed.transport;
  }
  if (self.didAttemptLoad && self.componentView != nil) {
    [self _connectPairedHost];
  }
}

- (void)disconnectPairedIfNeeded {
  [self.componentView disconnectPaired];
}

- (void)_connectPairedHost {
  NSString *resolvedEndpoint = self.pairEndpoint;
  if (self.pairTransport.length > 0) {
    const auto rewritten = nve::apple::apply_pair_transport(
        resolvedEndpoint.UTF8String ?: "", self.pairTransport.UTF8String ?: "");
    if (!rewritten.has_value()) {
      NSString *msg = [NSString
          stringWithFormat:@"NveIosShell: Invalid --pair-transport '%@' (expected auto|quic|wt|wss|ws)",
                           self.pairTransport];
      NSLog(@"%@", msg);
      [self _showErrorMessage:msg];
      return;
    }
    resolvedEndpoint = [NSString stringWithUTF8String:rewritten->c_str()];
  }
  NSError *err = nil;
  if (![self.componentView connectPairedEndpoint:resolvedEndpoint
                                           token:(self.pairToken ?: @"")
                                       surfaceId:self.surfaceId
                                           error:&err]) {
    NSString *detail = err.localizedDescription ?: @"unknown error";
    NSString *msg =
        [NSString stringWithFormat:@"NveIosShell: Failed to connect paired host %@: %@",
                                   resolvedEndpoint,
                                   detail];
    NSLog(@"%@", msg);
    [self _showErrorMessage:msg];
    return;
  }
  self.errorLabel.hidden = YES;
  NSLog(@"NveIosShell: Connected paired host %@ (nodes=%u)",
        resolvedEndpoint,
        self.componentView.nveNodeCount);
}

- (void)_loadBundledApp {
  if (self.pairEndpoint.length > 0) {
    [self _connectPairedHost];
    return;
  }

  NSString *dylibPath = NveResolveAppDylibPath();
  if (dylibPath.length == 0) {
    NSString *msg =
        @"NveIosShell: No app dylib found. Pass --pair-connect / NVE_PAIR_CONNECT or bundle libnve_app.dylib.";
    NSLog(@"%@", msg);
    [self _showErrorMessage:msg];
    return;
  }

  NSError *err = nil;
  if (![self.componentView loadNveDylibAtPath:dylibPath surfaceId:self.surfaceId error:&err]) {
    NSString *detail = err.localizedDescription ?: @"unknown error";
    NSString *msg =
        [NSString stringWithFormat:@"NveIosShell: Failed to load NVE dylib at %@: %@",
                                   dylibPath,
                                   detail];
    NSLog(@"%@", msg);
    [self _showErrorMessage:msg];
    return;
  }

  self.errorLabel.hidden = YES;
  NSLog(@"NveIosShell: Loaded NVE app dylib %@ (nodes=%u)",
        dylibPath,
        self.componentView.nveNodeCount);
}

- (void)_showErrorMessage:(NSString *)message {
  if (self.errorLabel == nil) {
    UILabel *label = [[UILabel alloc] initWithFrame:CGRectZero];
    label.translatesAutoresizingMaskIntoConstraints = NO;
    label.numberOfLines = 0;
    label.textAlignment = NSTextAlignmentCenter;
    label.textColor = [UIColor colorWithRed:0.97 green:0.44 blue:0.44 alpha:1.0];
    label.font = [UIFont monospacedSystemFontOfSize:13.0 weight:UIFontWeightMedium];
    [self.view addSubview:label];
    UILayoutGuide *safeArea = self.view.safeAreaLayoutGuide;
    [NSLayoutConstraint activateConstraints:@[
      [label.centerYAnchor constraintEqualToAnchor:safeArea.centerYAnchor],
      [label.leadingAnchor constraintEqualToAnchor:safeArea.leadingAnchor constant:24.0],
      [label.trailingAnchor constraintEqualToAnchor:safeArea.trailingAnchor constant:-24.0],
    ]];
    self.errorLabel = label;
  }
  self.errorLabel.hidden = NO;
  self.errorLabel.text = message;
}

@end

@class NveIosSceneDelegate;

@interface NveIosAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, readonly) NSUInteger activeSceneCount;
- (uint32_t)registerSceneDelegate:(NveIosSceneDelegate *)sceneDelegate
                       forSession:(UISceneSession *)session;
- (void)unregisterSceneForSurfaceId:(uint32_t)surfaceId session:(UISceneSession *)session;
- (nullable NveIosSceneDelegate *)sceneDelegateForSurfaceId:(uint32_t)surfaceId;
- (NSArray<NSNumber *> *)activeSurfaceIds;
@end

@interface NveIosSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong, nullable) UIWindow *window;
@property (nonatomic, assign, readonly) uint32_t surfaceId;
@end

@implementation NveIosSceneDelegate

- (void)scene:(UIScene *)scene
    willConnectToSession:(UISceneSession *)session
                 options:(UISceneConnectionOptions *)connectionOptions {
  if (![scene isKindOfClass:[UIWindowScene class]]) {
    return;
  }

  id<UIApplicationDelegate> appDelegate = UIApplication.sharedApplication.delegate;
  if ([appDelegate isKindOfClass:[NveIosAppDelegate class]]) {
    _surfaceId = [(NveIosAppDelegate *)appDelegate registerSceneDelegate:self forSession:session];
  } else {
    _surfaceId = NveAllocateIosSurfaceId();
  }

  UIWindowScene *windowScene = (UIWindowScene *)scene;
  UIWindow *window = [[UIWindow alloc] initWithWindowScene:windowScene];
  window.backgroundColor = NveDarkSlateColor();
  NveIosRootViewController *rootVC = [[NveIosRootViewController alloc] initWithSurfaceId:_surfaceId];
  for (UIOpenURLContext *ctx in connectionOptions.URLContexts) {
    [rootVC applyPairURL:ctx.URL];
  }
  window.rootViewController = rootVC;
  self.window = window;
  [window makeKeyAndVisible];
}

- (void)sceneDidDisconnect:(UIScene *)scene {
  UIViewController *root = self.window.rootViewController;
  if ([root isKindOfClass:[NveIosRootViewController class]]) {
    [(NveIosRootViewController *)root disconnectPairedIfNeeded];
  }
  id<UIApplicationDelegate> appDelegate = UIApplication.sharedApplication.delegate;
  if ([appDelegate isKindOfClass:[NveIosAppDelegate class]] && _surfaceId != 0u) {
    [(NveIosAppDelegate *)appDelegate unregisterSceneForSurfaceId:_surfaceId session:scene.session];
  }
  self.window = nil;
}

- (void)scene:(UIScene *)scene openURLContexts:(NSSet<UIOpenURLContext *> *)URLContexts {
  (void)scene;
  UIViewController *root = self.window.rootViewController;
  if (![root isKindOfClass:[NveIosRootViewController class]]) {
    return;
  }
  NveIosRootViewController *rootVC = (NveIosRootViewController *)root;
  for (UIOpenURLContext *ctx in URLContexts) {
    [rootVC applyPairURL:ctx.URL];
  }
}

@end

@implementation NveIosAppDelegate {
  NSMutableDictionary<NSNumber *, NveIosSceneDelegate *> *_activeScenes;
  NSMutableDictionary<NSString *, NSNumber *> *_sessionSurfaceIds;
}

- (instancetype)init {
  self = [super init];
  if (self) {
    _activeScenes = [NSMutableDictionary dictionary];
    _sessionSurfaceIds = [NSMutableDictionary dictionary];
  }
  return self;
}

- (NSUInteger)activeSceneCount {
  return _activeScenes.count;
}

- (uint32_t)registerSceneDelegate:(NveIosSceneDelegate *)sceneDelegate
                       forSession:(UISceneSession *)session {
  NSString *persistentId = session.persistentIdentifier;
  NSNumber *existing = persistentId.length > 0 ? _sessionSurfaceIds[persistentId] : nil;
  const uint32_t surfaceId =
      existing != nil ? existing.unsignedIntValue : NveAllocateIosSurfaceId();
  if (persistentId.length > 0) {
    _sessionSurfaceIds[persistentId] = @(surfaceId);
  }
  _activeScenes[@(surfaceId)] = sceneDelegate;
  return surfaceId;
}

- (void)unregisterSceneForSurfaceId:(uint32_t)surfaceId session:(UISceneSession *)session {
  [_activeScenes removeObjectForKey:@(surfaceId)];
  if (session.persistentIdentifier.length > 0) {
    [_sessionSurfaceIds removeObjectForKey:session.persistentIdentifier];
  }
}

- (nullable NveIosSceneDelegate *)sceneDelegateForSurfaceId:(uint32_t)surfaceId {
  return _activeScenes[@(surfaceId)];
}

- (NSArray<NSNumber *> *)activeSurfaceIds {
  return [_activeScenes.allKeys sortedArrayUsingSelector:@selector(compare:)];
}

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary<UIApplicationLaunchOptionsKey, id> *)launchOptions {
  (void)application;
  (void)launchOptions;
  return YES;
}

- (void)application:(UIApplication *)application
    didDiscardSceneSessions:(NSSet<UISceneSession *> *)sceneSessions {
  (void)application;
  for (UISceneSession *session in sceneSessions) {
    NSString *persistentId = session.persistentIdentifier;
    if (persistentId.length > 0) {
      NSNumber *sid = _sessionSurfaceIds[persistentId];
      if (sid != nil) {
        [_activeScenes removeObjectForKey:sid];
        [_sessionSurfaceIds removeObjectForKey:persistentId];
      }
    }
  }
}

- (void)applicationWillTerminate:(UIApplication *)application {
  for (UIScene *scene in application.connectedScenes) {
    if (![scene isKindOfClass:[UIWindowScene class]]) {
      continue;
    }
    UIWindowScene *windowScene = (UIWindowScene *)scene;
    for (UIWindow *window in windowScene.windows) {
      UIViewController *root = window.rootViewController;
      if ([root isKindOfClass:[NveIosRootViewController class]]) {
        [(NveIosRootViewController *)root disconnectPairedIfNeeded];
      }
    }
  }
}

- (UISceneConfiguration *)application:(UIApplication *)application
    configurationForConnectingSceneSession:(UISceneSession *)connectingSceneSession
                                   options:(UISceneConnectionOptions *)options {
  (void)application;
  (void)options;
  UISceneConfiguration *config =
      [[UISceneConfiguration alloc] initWithName:@"Default Configuration"
                                     sessionRole:connectingSceneSession.role];
  config.delegateClass = [NveIosSceneDelegate class];
  return config;
}

@end

int main(int argc, char *argv[]) {
  @autoreleasepool {
    return UIApplicationMain(argc, argv, nil, NSStringFromClass([NveIosAppDelegate class]));
  }
}
