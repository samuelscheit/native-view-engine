#import "NveMacFabricHost.h"

#include "nve_rn_component.hpp"
#include <dlfcn.h>

@implementation NveMacFabricHostController {
  NveFabricHostHandle _fabricHost;
  NveSurfaceConfig _config;
  NveFrameStats _lastStats;
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
    _appHandle = nullptr;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = nullptr;
  }
  if (_fabricHost) {
    nve_fabric_host_destroy(_fabricHost);
    _fabricHost = nullptr;
  }
}

- (NveFabricHostHandle)fabricHost {
  return _fabricHost;
}

- (NveSurfaceConfig)surfaceConfig {
  return _config;
}

- (NveFrameStats)lastFrameStats {
  return _lastStats;
}

- (uint32_t)nodeCount {
  return _fabricHost ? nve_fabric_host_node_count(_fabricHost) : 0;
}

- (BOOL)isAppLoaded {
  return _appHandle != nullptr;
}

- (BOOL)attachInterface:(NveGetInterfaceFn)getInterface {
  NveModule mod = nve_resolve_module(getInterface);
  if (mod.surface == nullptr || mod.surface->create == nullptr) {
    return NO;
  }
  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
    _appHandle = nullptr;
  }
  _module = mod;
  _appHandle = _module.surface->create(_config.surface_id);
  if (!_appHandle) {
    return NO;
  }
  [self renderNextFrame];
  return YES;
}

- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError * _Nullable * _Nullable)error {
  void *handle = dlopen(dylibPath.UTF8String, RTLD_NOW | RTLD_LOCAL);
  if (!handle) {
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.shell"
                                   code:1
                               userInfo:@{NSLocalizedDescriptionKey : @(dlerror() ?: "dlopen failed")}];
    }
    return NO;
  }
  auto getIface = reinterpret_cast<NveGetInterfaceFn>(dlsym(handle, "nve_get_interface"));
  if (!getIface) {
    dlclose(handle);
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.shell"
                                   code:2
                               userInfo:@{NSLocalizedDescriptionKey : @"Missing nve_get_interface symbol"}];
    }
    return NO;
  }
  NveModule mod = nve_resolve_module(getIface);
  if (!mod.surface || !mod.surface->create) {
    dlclose(handle);
    if (error) {
      *error = [NSError errorWithDomain:@"nve.macos.shell"
                                   code:3
                               userInfo:@{NSLocalizedDescriptionKey : @"Missing NVE_IFACE_SURFACE interface"}];
    }
    return NO;
  }

  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
    _appHandle = nullptr;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = nullptr;
  }
  _dylibHandle = handle;
  return [self attachInterface:getIface];
}

- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath error:(NSError * _Nullable * _Nullable)error {
  NSData *savedState = nil;
  if (_appHandle && _module.dev && _module.dev->snapshot_state) {
    NveBufferSlice s = _module.dev->snapshot_state(_appHandle);
    if (s.ptr && s.len > 0) {
      savedState = [NSData dataWithBytes:s.ptr length:s.len];
    }
  }
  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
    _appHandle = nullptr;
  }
  if (_dylibHandle) {
    dlclose(_dylibHandle);
    _dylibHandle = nullptr;
  }
  if (![self loadRustAppDylibAtPath:dylibPath error:error]) {
    return NO;
  }
  if (savedState.length > 0 && _module.dev && _module.dev->restore_state) {
    NveBufferSlice slice = {
        reinterpret_cast<const uint8_t *>(savedState.bytes),
        savedState.length,
    };
    _module.dev->restore_state(_appHandle, slice);
    [self renderNextFrame];
  }
  return YES;
}

- (void)setSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  _config.width = width;
  _config.height = height;
  if (scaleFactor > 0.0f) {
    _config.scale_factor = scaleFactor;
  }
  if (_fabricHost) {
    nve::fabric_host_set_surface_size(_fabricHost, _config.width, _config.height, _config.scale_factor);
  }
}

- (int32_t)renderNextFrame {
  if (!_appHandle || !_module.surface || !_module.surface->render) return -100;
  NveBufferSlice cmd = _module.surface->render(_appHandle, _frameId++);
  NveFrameStats stats{};
  const int32_t rc = nve_fabric_host_submit_frame(_fabricHost, cmd, &stats);
  _lastStats = stats;
  return rc;
}

- (BOOL)applyHotPatchJson:(NSString *)patchJson {
  if (!_appHandle || !_module.dev || !_module.dev->apply_hot_patch) return NO;
  NSData *utf8 = [patchJson dataUsingEncoding:NSUTF8StringEncoding];
  if (!utf8) return NO;
  NveBufferSlice slice = {reinterpret_cast<const uint8_t *>(utf8.bytes), utf8.length};
  if (_module.dev->apply_hot_patch(_appHandle, slice) != 0) {
    [self renderNextFrame];
    return YES;
  }
  return NO;
}

@end
