/// Native View Engine (NVE) Flutter host bindings and widgets.
///
/// Crisp barrel facade re-exporting:
/// - Generated C-ABI `dart:ffi` structs (`nve_core_abi.generated.dart`, `nve_scene_abi.generated.dart`)
/// - Compact binary `NveBinValue` zero-copy codec (`nve_bin_value.dart`)
/// - Direct FFI props & event writers (`direct_ffi.dart`)
/// - Strongly typed `EventRing` codec (`event_codec.dart`)
/// - Managed `nve_get_interface` instance wrapper (`app_instance.dart`)
/// - Imperative controller & `NveChannelBus` client (`channel_client.dart`)
/// - Live pairing client & surface (`nve_pair_client.dart`, `nve_paired_surface.dart`)
library nve_flutter;

export 'nve_hybrid_runtime.dart';
export 'src/app_instance.dart';
export 'src/channel_client.dart';
export 'src/event_codec.dart';
export 'src/nve_pair_client.dart';
export 'src/nve_paired_surface.dart';
export 'src/nve_scene_abi.generated.dart';
