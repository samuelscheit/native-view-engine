#import "NveMacWindowManager.h"
#import "NveAppleComponentHost.h"

#include <algorithm>
#include <vector>

@interface NveMacManagedWindowEntry : NSObject
@property (nonatomic, assign) uint32_t surfaceId;
@property (nonatomic, assign) uint32_t parentSurfaceId;
@property (nonatomic, assign) BOOL modal;
@property (nonatomic, assign) BOOL cascadeClose;
@property (nonatomic, assign) BOOL alwaysOnTop;
@property (nonatomic, strong) NSWindow *window;
@property (nonatomic, strong) RCTNveMacComponentView *componentView;
@property (nonatomic, strong) NSMutableArray<NSNumber *> *childSurfaceIds;
@end

@implementation NveMacManagedWindowEntry
- (instancetype)init {
  self = [super init];
  if (self) {
    _childSurfaceIds = [NSMutableArray array];
  }
  return self;
}
@end

@implementation NveMacWindowManager {
  NSMutableDictionary<NSNumber *, NveMacManagedWindowEntry *> *_entries;
  uint32_t _focusedSurfaceId;
  BOOL _closingInternal;
}

- (instancetype)init {
  self = [super init];
  if (self) {
    _entries = [NSMutableDictionary dictionary];
    _focusedSurfaceId = 0u;
    _closingInternal = NO;
  }
  return self;
}

- (void)dealloc {
  [self closeAllWindows];
}

- (NSUInteger)windowCount {
  return _entries.count;
}

- (uint32_t)focusedSurfaceId {
  return _focusedSurfaceId;
}

- (nullable RCTNveMacComponentView *)openWindowWithSurfaceId:(uint32_t)surfaceId
                                                       title:(NSString *)title
                                                       frame:(NSRect)frame
                                                     present:(BOOL)present {
  const NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                  NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
  return [self openWindowWithSurfaceId:surfaceId
                                 title:title
                                 frame:frame
                             styleMask:style
                       parentSurfaceId:0u
                                 modal:NO
                           alwaysOnTop:NO
                          cascadeClose:NO
                               present:present];
}

- (nullable RCTNveMacComponentView *)openWindowWithSurfaceId:(uint32_t)surfaceId
                                                       title:(NSString *)title
                                                       frame:(NSRect)frame
                                                   styleMask:(NSWindowStyleMask)styleMask
                                             parentSurfaceId:(uint32_t)parentSurfaceId
                                                       modal:(BOOL)modal
                                                 alwaysOnTop:(BOOL)alwaysOnTop
                                                cascadeClose:(BOOL)cascadeClose
                                                     present:(BOOL)present {
  if (surfaceId == 0u || _entries[@(surfaceId)] != nil) {
    return nil;
  }
  NveMacManagedWindowEntry *parentEntry = nil;
  if (parentSurfaceId != 0u) {
    parentEntry = _entries[@(parentSurfaceId)];
    if (parentEntry == nil) {
      return nil;
    }
  }

  const NSRect safeFrame = NSMakeRect(
      frame.origin.x,
      frame.origin.y,
      frame.size.width > 0.0 ? frame.size.width : 640.0,
      frame.size.height > 0.0 ? frame.size.height : 420.0);

  NSWindow *window = [[NSWindow alloc] initWithContentRect:safeFrame
                                                 styleMask:styleMask
                                                   backing:NSBackingStoreBuffered
                                                     defer:NO];
  window.releasedWhenClosed = NO;
  window.title = title ?: @"NVE macOS Window";
  window.delegate = self;
  if (alwaysOnTop) {
    window.level = NSFloatingWindowLevel;
  }

  const NSRect contentBounds = NSMakeRect(0.0, 0.0, safeFrame.size.width, safeFrame.size.height);
  RCTNveMacComponentView *rootView = [[RCTNveMacComponentView alloc] initWithFrame:contentBounds];
  rootView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
  window.contentView = rootView;

  NveMacManagedWindowEntry *entry = [[NveMacManagedWindowEntry alloc] init];
  entry.surfaceId = surfaceId;
  entry.parentSurfaceId = parentSurfaceId;
  entry.modal = modal;
  entry.cascadeClose = cascadeClose || modal;
  entry.alwaysOnTop = alwaysOnTop;
  entry.window = window;
  entry.componentView = rootView;

  _entries[@(surfaceId)] = entry;
  if (parentEntry != nil) {
    [parentEntry.childSurfaceIds addObject:@(surfaceId)];
  }
  _focusedSurfaceId = surfaceId;

  if (present) {
    if (safeFrame.origin.x == 0.0 && safeFrame.origin.y == 0.0) {
      [window center];
    }
    [window makeKeyAndOrderFront:nil];
  }
  return rootView;
}

- (nullable NSWindow *)windowForSurfaceId:(uint32_t)surfaceId {
  return _entries[@(surfaceId)].window;
}

- (nullable RCTNveMacComponentView *)componentViewForSurfaceId:(uint32_t)surfaceId {
  return _entries[@(surfaceId)].componentView;
}

- (NSArray<NSNumber *> *)activeSurfaceIds {
  NSArray<NSNumber *> *keys = _entries.allKeys;
  return [keys sortedArrayUsingSelector:@selector(compare:)];
}

- (uint32_t)activeModalBlockerForSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil) {
    return 0u;
  }
  for (NSNumber *childNum in [entry.childSurfaceIds reverseObjectEnumerator]) {
    const uint32_t childId = childNum.unsignedIntValue;
    const uint32_t deeper = [self activeModalBlockerForSurfaceId:childId];
    if (deeper != 0u) {
      return deeper;
    }
    NveMacManagedWindowEntry *child = _entries[childNum];
    if (child != nil && child.modal) {
      return childId;
    }
  }
  return 0u;
}

- (BOOL)focusWindowWithSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil) {
    return NO;
  }
  if ([self activeModalBlockerForSurfaceId:surfaceId] != 0u) {
    return NO;
  }
  _focusedSurfaceId = surfaceId;
  if (entry.window.visible) {
    [entry.window makeKeyAndOrderFront:nil];
  }
  return YES;
}

- (BOOL)setTitle:(NSString *)title forSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil) {
    return NO;
  }
  entry.window.title = title ?: @"";
  return YES;
}

- (BOOL)setFrame:(NSRect)frame forSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil || frame.size.width <= 0.0 || frame.size.height <= 0.0) {
    return NO;
  }
  [entry.window setFrame:frame display:NO];
  return YES;
}

- (BOOL)setAlwaysOnTop:(BOOL)alwaysOnTop forSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil) {
    return NO;
  }
  entry.alwaysOnTop = alwaysOnTop;
  entry.window.level = alwaysOnTop ? NSFloatingWindowLevel : NSNormalWindowLevel;
  return YES;
}

- (void)_collectCascadeOrderForSurfaceId:(uint32_t)surfaceId outOrder:(std::vector<uint32_t> &)out {
  NveMacManagedWindowEntry *entry = _entries[@(surfaceId)];
  if (entry == nil) {
    return;
  }
  for (NSNumber *childNum in entry.childSurfaceIds) {
    NveMacManagedWindowEntry *child = _entries[childNum];
    if (child != nil && child.cascadeClose) {
      [self _collectCascadeOrderForSurfaceId:child.surfaceId outOrder:out];
    }
  }
  out.push_back(surfaceId);
}

- (BOOL)requestCloseSurfaceId:(uint32_t)surfaceId {
  if (_entries[@(surfaceId)] == nil) {
    return NO;
  }
  if (self.shouldCloseWindow != nil && !self.shouldCloseWindow(surfaceId)) {
    return NO;
  }
  return [self closeWindowWithSurfaceId:surfaceId];
}

- (BOOL)closeWindowWithSurfaceId:(uint32_t)surfaceId {
  NveMacManagedWindowEntry *rootEntry = _entries[@(surfaceId)];
  if (rootEntry == nil) {
    return NO;
  }
  const uint32_t fallbackParent = rootEntry.parentSurfaceId;
  std::vector<uint32_t> order;
  [self _collectCascadeOrderForSurfaceId:surfaceId outOrder:order];

  const BOOL prevClosing = _closingInternal;
  _closingInternal = YES;

  BOOL focusClosed = NO;
  std::vector<uint32_t> closedIds;
  closedIds.reserve(order.size());

  for (uint32_t sid : order) {
    NveMacManagedWindowEntry *entry = _entries[@(sid)];
    if (entry == nil) {
      continue;
    }
    if (entry.parentSurfaceId != 0u) {
      NveMacManagedWindowEntry *parent = _entries[@(entry.parentSurfaceId)];
      [parent.childSurfaceIds removeObject:@(sid)];
    }
    for (NSNumber *cid in entry.childSurfaceIds) {
      NveMacManagedWindowEntry *remainingChild = _entries[cid];
      if (remainingChild != nil) {
        remainingChild.parentSurfaceId = 0u;
      }
    }
    [_entries removeObjectForKey:@(sid)];
    if (_focusedSurfaceId == sid) {
      _focusedSurfaceId = 0u;
      focusClosed = YES;
    }

    NveAppleComponentHost *host = [NveAppleComponentHost hostForRootView:entry.componentView];
    [host disconnectPaired];
    entry.window.delegate = nil;
    [entry.window close];
    closedIds.push_back(sid);
  }

  _closingInternal = prevClosing;

  if (focusClosed) {
    if (fallbackParent != 0u && _entries[@(fallbackParent)] != nil &&
        [self activeModalBlockerForSurfaceId:fallbackParent] == 0u) {
      _focusedSurfaceId = fallbackParent;
    } else {
      NSNumber *last = [self activeSurfaceIds].lastObject;
      _focusedSurfaceId = last != nil ? last.unsignedIntValue : 0u;
    }
  }

  if (self.onWindowClosed != nil) {
    for (uint32_t sid : closedIds) {
      self.onWindowClosed(sid);
    }
  }
  if (_entries.count == 0u && self.onAllWindowsClosed != nil) {
    self.onAllWindowsClosed();
  }
  return YES;
}

- (void)closeAllWindows {
  NSArray<NSNumber *> *ids = [self activeSurfaceIds];
  for (NSNumber *sid in ids) {
    if (_entries[sid] != nil) {
      [self closeWindowWithSurfaceId:sid.unsignedIntValue];
    }
  }
}

- (uint32_t)_surfaceIdForWindow:(NSWindow *)window {
  for (NSNumber *key in _entries) {
    if (_entries[key].window == window) {
      return key.unsignedIntValue;
    }
  }
  return 0u;
}

#pragma mark - NSWindowDelegate

- (BOOL)windowShouldClose:(NSWindow *)sender {
  const uint32_t sid = [self _surfaceIdForWindow:sender];
  if (sid == 0u) {
    return YES;
  }
  if (self.shouldCloseWindow != nil && !self.shouldCloseWindow(sid)) {
    return NO;
  }
  [self closeWindowWithSurfaceId:sid];
  return NO;
}

- (void)windowWillClose:(NSNotification *)notification {
  if (_closingInternal) {
    return;
  }
  NSWindow *sender = notification.object;
  if (![sender isKindOfClass:[NSWindow class]]) {
    return;
  }
  const uint32_t sid = [self _surfaceIdForWindow:sender];
  if (sid != 0u) {
    [self closeWindowWithSurfaceId:sid];
  }
}

- (void)windowDidBecomeKey:(NSNotification *)notification {
  NSWindow *sender = notification.object;
  if (![sender isKindOfClass:[NSWindow class]]) {
    return;
  }
  const uint32_t sid = [self _surfaceIdForWindow:sender];
  if (sid == 0u) {
    return;
  }
  const uint32_t blocker = [self activeModalBlockerForSurfaceId:sid];
  if (blocker != 0u) {
    NSWindow *modalWin = _entries[@(blocker)].window;
    if (modalWin != nil && modalWin.visible) {
      [modalWin makeKeyAndOrderFront:nil];
    }
    _focusedSurfaceId = blocker;
    return;
  }
  _focusedSurfaceId = sid;
}

@end
