/* Compiles the generated C ABI once inside SwiftPM (a C target needs one translation unit). */
#include "../../../host-cxx/include/nve/generated/nve_abi.generated.h"
#include "../../../host-cxx/include/nve/generated/nve_tables.generated.h"

uint32_t nve_abi_version(void) { return NVE_ABI_VERSION; }
