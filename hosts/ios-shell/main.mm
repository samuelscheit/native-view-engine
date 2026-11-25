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

}  // namespace

@interface NveIosRootViewController : UIViewController
@property (nonatomic, strong) RCTNveComponentView *componentView;
@property (nonatomic, strong, nullable) UILabel *errorLabel;
@property (nonatomic, copy, nullable) NSString *snapshotPath;
@property (nonatomic, assign) uint32_t snapshotSchemaVersion;
@property (nonatomic, assign) BOOL didAttemptLoad;
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

  NSString *dylibPath = NveResolveAppDylibPath();
  if (dylibPath.length == 0) {
    NSString *msg =
        @"NveIosShell: No app dylib found. Set NVE_APP_DYLIB or bundle libnve_app.dylib in the .app.";
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
  (void)connectionOptions;
  if (![scene isKindOfClass:[UIWindowScene class]]) {
    return;
  }

  UIWindowScene *windowScene = (UIWindowScene *)scene;
  UIWindow *window = [[UIWindow alloc] initWithWindowScene:windowScene];
  window.backgroundColor = NveDarkSlateColor();
  window.rootViewController = [[NveIosRootViewController alloc] init];
  self.window = window;
  [window makeKeyAndVisible];
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
        [(NveIosRootViewController *)root saveSnapshotIfPossible];
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
