#import "NveAppleFabricHost.h"
#import "NveAppleSupport.h"

#include "nve_rn_component.hpp"

#include <vector>

@implementation NveAppleFabricHostController {
  NveFabricHostHandle _fabricHost;
  NveSurfaceConfig _config;
  NveFrameStats _lastStats;
  NSString *_errorDomain;
  void *_dylibHandle;
  NveModule _module;
  NveAppHandle _appHandle;
  uint64_t _frameId;
}

+ (NSString *)defaultErrorDomain {
  return @"nve.apple.shell";
}

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config
                          errorDomain:(NSString *)errorDomain {
  self = [super init];
  if (self) {
    _config = config;
    _errorDomain = [errorDomain copy];
    _fabricHost = nve_fabric_host_create(config);
    nve::apple::install_native_text_measurer(_fabricHost);
    _frameId = 1;
  }
  return self;
}

- (instancetype)initWithSurfaceConfig:(NveSurfaceConfig)config {
  return [self initWithSurfaceConfig:config errorDomain:[[self class] defaultErrorDomain]];
}

- (void)dealloc {
  [self _teardownAppAndDylib];
  if (_fabricHost) {
    nve_fabric_host_destroy(_fabricHost);
    _fabricHost = nullptr;
  }
}

- (void)_teardownAppAndDylib {
  if (_appHandle && _module.surface && _module.surface->destroy) {
    _module.surface->destroy(_appHandle);
    _appHandle = nullptr;
  }
  _module = NveModule{};
  nve::apple::close_dylib(_dylibHandle);
}

- (BOOL)_installModule:(NveModule)mod
           dylibHandle:(void *)dylibHandle
         restoredState:(const std::vector<uint8_t> &)savedState
                 error:(NSError * _Nullable * _Nullable)error {
  if (!mod.surface || !mod.surface->create) {
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = [NSError errorWithDomain:_errorDomain
                                   code:static_cast<NSInteger>(
                                            nve::apple::DylibErrorCode::MissingSurfaceInterface)
                               userInfo:@{
                                 NSLocalizedDescriptionKey : @"Missing NVE_IFACE_SURFACE interface"
                               }];
    }
    return NO;
  }

  // Transactional swap: create the replacement app instance BEFORE tearing down the live app/dylib.
  NveAppHandle nextApp = mod.surface->create(_config.surface_id);
  if (!nextApp) {
    nve::apple::close_dylib(dylibHandle);
    if (error) {
      *error = [NSError errorWithDomain:_errorDomain
                                   code:static_cast<NSInteger>(
                                            nve::apple::DylibErrorCode::MissingSurfaceInterface)
                               userInfo:@{
                                 NSLocalizedDescriptionKey : @"Failed to initialize NVE app instance"
                               }];
    }
    return NO;
  }

  [self _teardownAppAndDylib];

  _dylibHandle = dylibHandle;
  _module = mod;
  _appHandle = nextApp;
  if (!savedState.empty() && _module.dev && _module.dev->restore_state) {
    const NveBufferSlice slice = {savedState.data(), savedState.size()};
    _module.dev->restore_state(_appHandle, slice);
  }
  [self renderNextFrame];
  return YES;
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
  const NveModule mod = nve_resolve_module(getInterface);
  return [self _installModule:mod dylibHandle:nullptr restoredState:{} error:nil];
}

- (BOOL)loadRustAppDylibAtPath:(NSString *)dylibPath
                         error:(NSError * _Nullable * _Nullable)error {
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                restoredState:{}
                        error:error];
}

- (BOOL)reloadRustAppDylibAtPath:(NSString *)dylibPath
                           error:(NSError * _Nullable * _Nullable)error {
  // Validate the replacement dylib before tearing down the live app so a failed reload
  // leaves the currently running app and its state intact.
  nve::apple::LoadedDylib loaded{};
  if (!nve::apple::open_nve_dylib(dylibPath, _errorDomain, &loaded, error)) {
    return NO;
  }

  std::vector<uint8_t> savedState;
  if (_appHandle && _module.dev && _module.dev->snapshot_state) {
    const NveBufferSlice s = _module.dev->snapshot_state(_appHandle);
    if (s.ptr && s.len > 0) {
      savedState.assign(s.ptr, s.ptr + s.len);
    }
  }
  return [self _installModule:loaded.module
                  dylibHandle:loaded.handle
                restoredState:savedState
                        error:error];
}

- (void)setSurfaceWidth:(float)width height:(float)height scaleFactor:(float)scaleFactor {
  _config.width = width;
  _config.height = height;
  if (scaleFactor > 0.0f) {
    _config.scale_factor = scaleFactor;
  }
  if (_fabricHost) {
    nve::fabric_host_set_surface_size(
        _fabricHost, _config.width, _config.height, _config.scale_factor);
  }
}

- (int32_t)renderNextFrame {
  if (!_appHandle || !_module.surface || !_module.surface->render) return -100;
  const NveBufferSlice cmd = _module.surface->render(_appHandle, _frameId++);
  NveFrameStats stats{};
  const int32_t rc = nve_fabric_host_submit_frame(_fabricHost, cmd, &stats);
  _lastStats = stats;
  return rc;
}

- (BOOL)applyHotPatch:(NSData *)patchNvp1 {
  if (!_appHandle || !_module.dev || !_module.dev->apply_hot_patch || !patchNvp1) return NO;
  const NveBufferSlice slice = {reinterpret_cast<const uint8_t *>(patchNvp1.bytes), patchNvp1.length};
  if (_module.dev->apply_hot_patch(_appHandle, slice) != 0) {
    [self renderNextFrame];
    return YES;
  }
  return NO;
}

@end
