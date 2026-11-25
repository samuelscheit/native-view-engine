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

}  // namespace

@interface NveIosRootViewController : UIViewController
@property (nonatomic, strong) RCTNveComponentView *componentView;
@property (nonatomic, strong, nullable) UILabel *errorLabel;
@property (nonatomic, assign) BOOL didAttemptLoad;
@end

@implementation NveIosRootViewController

- (UIStatusBarStyle)preferredStatusBarStyle {
  return UIStatusBarStyleLightContent;
}

- (void)viewDidLoad {
  [super viewDidLoad];
  self.view.backgroundColor = NveDarkSlateColor();

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

@interface NveIosAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, strong) UIWindow *window;
@end

@implementation NveIosAppDelegate

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary<UIApplicationLaunchOptionsKey, id> *)launchOptions {
  (void)application;
  (void)launchOptions;

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
  CGRect screenBounds = [UIScreen mainScreen].bounds;
#pragma clang diagnostic pop

  UIWindow *window = [[UIWindow alloc] initWithFrame:screenBounds];
  window.backgroundColor = NveDarkSlateColor();
  window.rootViewController = [[NveIosRootViewController alloc] init];
  self.window = window;
  [window makeKeyAndVisible];
  return YES;
}

@end

int main(int argc, char *argv[]) {
  @autoreleasepool {
    return UIApplicationMain(argc, argv, nil, NSStringFromClass([NveIosAppDelegate class]));
  }
}
