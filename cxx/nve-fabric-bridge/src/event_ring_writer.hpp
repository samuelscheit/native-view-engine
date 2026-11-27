#pragma once

#include "nve_event_batch.hpp"

namespace nve {

/// Binary `NVEV` v2 EventRing & zero-copy `NveEventBatch` writer backed by `EventBatchBuilder`.
using EventRingWriter = EventBatchBuilder;

} // namespace nve
