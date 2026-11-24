#import "NveFabricHost.h"
#include <dlfcn.h>

typedef NveAppVTable (*NveVTableFn)(void);

@implementation NveFabricHostController {
  NveFabricHostHandle _fabricHost;
  NveSurfaceConfig _config;
  void *_dylibHandle;
  NveAppVTable _vtable;
  NveAppHandle _appHandle;
  uint64_t _frameId;
}

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config {
  self = [super init];
  if (self) {
    _config = config;
    _fabricHost = nve_fabric_host_create(config);
    _frameId = 1;
  }
  return self;
}

- (void)dealloc {
  if (_appHandle && _vtable.destroy) {
    _vtable.destroy(_appHandle);
    _appHandle = NULL;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = NULL;
  }
  if (_fabricHost) {
    nve_fabric_host_destroy(_fabricHost);
    _fabricHost = NULL;
  }
}

- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  void *handle = dlopen(dylibPath.UTF8String, RTLD_NOW | RTLD_LOCAL);
  if (!handle) {
    if (error) {
      *error = [NSError errorWithDomain:@"nve.ios.shell"
                                   code:1
                               userInfo:@{NSLocalizedDescriptionKey : @(dlerror() ?: "dlopen failed")}];
    }
    return NO;
  }
  NveVTableFn fn = (NveVTableFn)dlsym(handle, "nve_app_vtable");
  if (!fn) {
    dlclose(handle);
    return NO;
  }
  NveAppVTable vt = fn();
  if (vt.abi_version != NVE_APP_ABI_VERSION) {
    dlclose(handle);
    return NO;
  }
  _dylibHandle = handle;
  _vtable = vt;
  _appHandle = _vtable.create(_config.surface_id);
  [self renderNextFrame];
  return YES;
}

- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  NSData *savedState = nil;
  if (_appHandle && _vtable.snapshot_state) {
    NveBufferSlice s = _vtable.snapshot_state(_appHandle);
    if (s.ptr && s.len > 0) {
      savedState = [NSData dataWithBytes:s.ptr length:s.len];
    }
    _vtable.destroy(_appHandle);
    _appHandle = NULL;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = NULL;
  }
  if (![self loadRustAppDylibAtPath:dylibPath error:error]) {
    return NO;
  }
  if (savedState.length > 0 && _vtable.restore_state) {
    NveBufferSlice slice = {(const uint8_t *)savedState.bytes, savedState.length};
    _vtable.restore_state(_appHandle, slice);
    [self renderNextFrame];
  }
  return YES;
}

- (void)renderNextFrame {
  if (!_appHandle || !_vtable.render) return;
  NveBufferSlice cmd = _vtable.render(_appHandle, _frameId++);
  NveFrameStats stats = {};
  nve_fabric_host_submit_frame(_fabricHost, cmd, &stats);
}

- (void)applyHotPatchJson:(NSString *)patchJson {
  if (!_appHandle || !_vtable.apply_hot_patch) return;
  NSData *utf8 = [patchJson dataUsingEncoding:NSUTF8StringEncoding];
  NveBufferSlice slice = {(const uint8_t *)utf8.bytes, utf8.length};
  if (_vtable.apply_hot_patch(_appHandle, slice) != 0) {
    [self renderNextFrame];
  }
}

@end
