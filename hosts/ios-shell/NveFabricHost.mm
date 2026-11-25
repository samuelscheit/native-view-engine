#import "NveFabricHost.h"
#include <dlfcn.h>

@implementation NveFabricHostController {
  NveFabricHostHandle _fabricHost;
  NveSurfaceConfig _config;
  void *_dylibHandle;
  NveModule _module;
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
  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
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
  NveGetInterfaceFn getIface = (NveGetInterfaceFn)dlsym(handle, "nve_get_interface");
  if (!getIface) {
    dlclose(handle);
    return NO;
  }
  NveModule mod = nve_resolve_module(getIface);
  if (!mod.surface || !mod.surface->create) {
    dlclose(handle);
    return NO;
  }
  _dylibHandle = handle;
  _module = mod;
  _appHandle = _module.surface->create(_config.surface_id);
  [self renderNextFrame];
  return YES;
}

- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError **)error {
  NSData *savedState = nil;
  if (_appHandle && _module.dev && _module.dev->snapshot_state) {
    NveBufferSlice s = _module.dev->snapshot_state(_appHandle);
    if (s.ptr && s.len > 0) {
      savedState = [NSData dataWithBytes:s.ptr length:s.len];
    }
  }
  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
    _appHandle = NULL;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = NULL;
  }
  if (![self loadRustAppDylibAtPath:dylibPath error:error]) {
    return NO;
  }
  if (savedState.length > 0 && _module.dev && _module.dev->restore_state) {
    NveBufferSlice slice = {(const uint8_t *)savedState.bytes, savedState.length};
    _module.dev->restore_state(_appHandle, slice);
    [self renderNextFrame];
  }
  return YES;
}

- (void)renderNextFrame {
  if (!_appHandle || !_module.surface || !_module.surface->render) return;
  NveBufferSlice cmd = _module.surface->render(_appHandle, _frameId++);
  NveFrameStats stats = {};
  nve_fabric_host_submit_frame(_fabricHost, cmd, &stats);
}

- (void)applyHotPatchJson:(NSString *)patchJson {
  if (!_appHandle || !_module.dev || !_module.dev->apply_hot_patch) return;
  NSData *utf8 = [patchJson dataUsingEncoding:NSUTF8StringEncoding];
  NveBufferSlice slice = {(const uint8_t *)utf8.bytes, utf8.length};
  if (_module.dev->apply_hot_patch(_appHandle, slice) != 0) {
    [self renderNextFrame];
  }
}

@end
