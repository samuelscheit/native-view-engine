#include "../../../../packages/core/host-cxx/include/nve/host/nve_runtime_abi.generated.h"

/* Translation unit ensuring SwiftPM produces CNveAbi.o cleanly when linking static archives. */
uint32_t nve_swiftui_runtime_abi_version(void) {
  return NVE_RUNTIME_ABI_VERSION;
}