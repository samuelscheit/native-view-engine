// Negative compile test: each `NVE_EXPECT_*` define must make this file fail to compile, and the
// plain file must compile (so the failure is the guarded misuse, not a broken include).
#include "nve/host/table_reader.hpp"
#include "nve/host/xxh64.hpp"

constexpr auto kValid = nve::host::field(3, 8, 8, NVE_FIELD_U32);
constexpr auto kValidKey = nve::host::Key64::of("nve.visual");

#if defined(NVE_EXPECT_BAD_FIELD_BITS)
constexpr auto kBad = nve::host::field(0, 24, 16, NVE_FIELD_U32);  // shift + bits > 32
#elif defined(NVE_EXPECT_BAD_FIELD_ZERO)
constexpr auto kBad = nve::host::field(0, 0, 0, NVE_FIELD_U32);  // zero width
#elif defined(NVE_EXPECT_BAD_FIELD_WORD)
constexpr auto kBad = nve::host::field(31, 0, 32, NVE_FIELD_STR);  // second string word past 32
#elif defined(NVE_EXPECT_BAD_KEY)
constexpr auto kBad = nve::host::Key64::of("\xED\xA0\x80");  // lone surrogate is not a Rust &str
#endif

int main() { return kValid.bits == 8 && kValidKey.value != 0 ? 0 : 1; }
