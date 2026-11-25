#import <UIKit/UIKit.h>

#import "RCTNveComponentView.h"

namespace {

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

NSString * _Nullable NveResolveSnapshotPath(void) {
  NSDictionary<NSString *, NSString *> *env = NSProcessInfo.processInfo.environment;
  NSString *disabled = env[@"NVE_SNAPSHOT_DISABLE"];
  if ([disabled isEqualToString:@"1"] || [disabled.lowercaseString isEqualToString:@"true"]) {
    return nil;
  }
  NSString *envPath = env[@"NVE_SNAPSHOT_PATH"];
  if (envPath.length > 0) {
    return envPath;
  }
  NSArray<NSString *> *caches =
      NSSearchPathForDirectoriesInDomains(NSCachesDirectory, NSUserDomainMask, YES);
  NSString *cacheDir = caches.firstObject;
  if (cacheDir.length == 0) {
    return nil;
  }
  return [cacheDir stringByAppendingPathComponent:@"nve_frame0.nvesnap"];
}

uint32_t NveResolveSnapshotSchemaVersion(void) {
  NSString *raw = NSProcessInfo.processInfo.environment[@"NVE_SNAPSHOT_SCHEMA"];
  if (raw.length > 0) {
    long parsed = strtol(raw.UTF8String, nullptr, 10);
    if (parsed >= 0) {
      return static_cast<uint32_t>(parsed);
    }
  }
  return 1u;
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
  for (NSUInteger i = 1; i + 1 < args.count; ++i) {
    NSString *flag = args[i];
    if ([flag isEqualToString:@"--pair-connect"]) {
      cfg.endpoint = args[++i];
    } else if ([flag isEqualToString:@"--pair-token"]) {
      cfg.token = args[++i];
    } else if ([flag isEqualToString:@"--pair-transport"]) {
      cfg.transport = args[++i];
    }
  }
  return cfg;
}

}  // namespace

@interface NveIosRootViewController : UIViewController
@property (nonatomic, strong) RCTNveComponentView *componentView;
@property (nonatomic, strong, nullable) UILabel *errorLabel;
@property (nonatomic, copy, nullable) NSString *snapshotPath;
@property (nonatomic, copy, nullable) NSString *pairEndpoint;
@property (nonatomic, copy, nullable) NSString *pairToken;
@property (nonatomic, copy, nullable) NSString *pairTransport;
@property (nonatomic, assign) uint32_t snapshotSchemaVersion;
@property (nonatomic, assign) BOOL didAttemptLoad;
- (void)applyPairURL:(NSURL *)url;
- (void)disconnectPairedIfNeeded;
- (void)saveSnapshotIfPossible;
@end

@implementation NveIosRootViewController

- (UIStatusBarStyle)preferredStatusBarStyle {
  return UIStatusBarStyleLightContent;
}

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = NveDarkSlateColor();
  self.snapshotPath = NveResolveSnapshotPath();
  self.snapshotSchemaVersion = NveResolveSnapshotSchemaVersion();
  if (self.pairEndpoint.length == 0) {
    const NveIosPairConfig launchPair = NveResolveLaunchPairConfig();
    self.pairEndpoint = launchPair.endpoint;
    self.pairToken = launchPair.token;
    self.pairTransport = launchPair.transport;
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
  self.pairTransport = parsed.transport;
  if (self.didAttemptLoad && self.componentView != nil) {
    [self _connectPairedHost];
  }
}

- (void)disconnectPairedIfNeeded {
  [self.componentView disconnectPaired];
}

- (void)_connectPairedHost {
  NSError *err = nil;
  if (![self.componentView connectPairedEndpoint:self.pairEndpoint
                                           token:(self.pairToken ?: @"")
                                       transport:self.pairTransport
                                       surfaceId:1
                                           error:&err]) {
    NSString *detail = err.localizedDescription ?: @"unknown error";
    NSString *msg =
        [NSString stringWithFormat:@"NveIosShell: Failed to connect paired host %@: %@",
                                   self.pairEndpoint,
                                   detail];
    NSLog(@"%@", msg);
    [self _showErrorMessage:msg];
    return;
  }
  self.errorLabel.hidden = YES;
  NSLog(@"NveIosShell: Connected paired host %@ (nodes=%u)",
        self.pairEndpoint,
        self.componentView.nveNodeCount);
}

- (void)_loadBundledApp {
  if (self.snapshotPath.length > 0 &&
      [[NSFileManager defaultManager] fileExistsAtPath:self.snapshotPath]) {
    uint64_t restoreUs = 0;
    if ([self.componentView replayFrame0SnapshotAtPath:self.snapshotPath
                                 expectedSchemaVersion:self.snapshotSchemaVersion
                                         restoreTimeUs:&restoreUs]) {
      NSLog(@"NveIosShell: Instant-boot Frame-0 mmap restore from %@ in %llu us (nodes=%u)",
            self.snapshotPath,
            static_cast<unsigned long long>(restoreUs),
            self.componentView.nveNodeCount);
    }
  }

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
  if (![self.componentView loadNveDylibAtPath:dylibPath surfaceId:1 error:&err]) {
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

- (void)saveSnapshotIfPossible {
  if (self.snapshotPath.length == 0 || self.componentView == nil) {
    return;
  }
  NSString *parentDir = [self.snapshotPath stringByDeletingLastPathComponent];
  if (parentDir.length > 0) {
    [[NSFileManager defaultManager] createDirectoryAtPath:parentDir
                              withIntermediateDirectories:YES
                                               attributes:nil
                                                    error:nil];
  }
  if ([self.componentView saveSnapshotAtPath:self.snapshotPath
                               schemaVersion:self.snapshotSchemaVersion]) {
    NSLog(@"NveIosShell: Saved Frame-0 + state snapshot to %@ (schema v%u)",
          self.snapshotPath,
          self.snapshotSchemaVersion);
  }
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

@interface NveIosSceneDelegate : UIResponder <UIWindowSceneDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation NveIosSceneDelegate

- (void)scene:(UIScene *)scene
    willConnectToSession:(UISceneSession *)session
                 options:(UISceneConnectionOptions *)connectionOptions {
  (void)session;
  if (![scene isKindOfClass:[UIWindowScene class]]) {
    return;
  }

  UIWindowScene *windowScene = (UIWindowScene *)scene;
  UIWindow *window = [[UIWindow alloc] initWithWindowScene:windowScene];
  window.backgroundColor = NveDarkSlateColor();
  NveIosRootViewController *rootVC = [[NveIosRootViewController alloc] init];
  for (UIOpenURLContext *ctx in connectionOptions.URLContexts) {
    [rootVC applyPairURL:ctx.URL];
  }
  window.rootViewController = rootVC;
  self.window = window;
  [window makeKeyAndVisible];
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

- (void)sceneDidEnterBackground:(UIScene *)scene {
  (void)scene;
  UIViewController *root = self.window.rootViewController;
  if ([root isKindOfClass:[NveIosRootViewController class]]) {
    [(NveIosRootViewController *)root saveSnapshotIfPossible];
  }
}

@end

@interface NveIosAppDelegate : UIResponder <UIApplicationDelegate>
@end

@implementation NveIosAppDelegate

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary<UIApplicationLaunchOptionsKey, id> *)launchOptions {
  (void)application;
  (void)launchOptions;
  return YES;
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
        NveIosRootViewController *rootVC = (NveIosRootViewController *)root;
        [rootVC saveSnapshotIfPossible];
        [rootVC disconnectPairedIfNeeded];
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
