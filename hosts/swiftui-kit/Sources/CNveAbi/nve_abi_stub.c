#include "nve_abi.h"

/* Translation unit ensuring SwiftPM produces CNveAbi.o cleanly when linking static archives. */
uint32_t nve_swiftui_abi_version(void) {
  return NVE_ABI_VERSION;
}
