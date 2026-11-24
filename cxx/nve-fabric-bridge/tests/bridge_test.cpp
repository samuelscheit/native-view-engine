#include "nve_abi.h"

#include <cassert>
#include <cstring>
#include <iostream>
#include <vector>

namespace {

template <typename T>
void push(std::vector<uint8_t>& buf, T value) {
  const auto* p = reinterpret_cast<const uint8_t*>(&value);
  buf.insert(buf.end(), p, p + sizeof(T));
}

void push_str(std::vector<uint8_t>& buf, const char* s) {
  const size_t n = std::strlen(s);
  buf.insert(buf.end(), reinterpret_cast<const uint8_t*>(s), reinterpret_cast<const uint8_t*>(s) + n);
}

} // namespace

int main() {
  NveSurfaceConfig cfg{.surface_id = 1, .width = 393.0f, .height = 852.0f, .scale_factor = 3.0f};
  NveFabricHostHandle host = nve_fabric_host_create(cfg);
  assert(host != nullptr);
  assert(nve_fabric_host_node_count(host) == 1);

  std::vector<uint8_t> cmd;
  push<uint32_t>(cmd, NVE_ABI_MAGIC);
  push<uint16_t>(cmd, NVE_ABI_VERSION);
  push<uint16_t>(cmd, 0);
  push<uint32_t>(cmd, 1); /* surface_id */
  push<uint64_t>(cmd, 1); /* frame_id */
  push<uint32_t>(cmd, 4); /* op_count */

  /* 1. CreateNode(2, View) */
  push<uint8_t>(cmd, NVE_OP_CREATE_NODE);
  push<uint32_t>(cmd, 2);
  push<uint16_t>(cmd, NVE_COMP_VIEW);
  push<uint16_t>(cmd, 0);

  /* 2. CreateNode(3, Text) */
  push<uint8_t>(cmd, NVE_OP_CREATE_NODE);
  push<uint32_t>(cmd, 3);
  push<uint16_t>(cmd, NVE_COMP_TEXT);
  push<uint16_t>(cmd, 0);

  /* 3. SetText(3, "Hello from C++ Fabric Bridge") */
  const char* msg = "Hello from C++ Fabric Bridge";
  push<uint8_t>(cmd, NVE_OP_SET_TEXT);
  push<uint32_t>(cmd, 3);
  push<uint32_t>(cmd, static_cast<uint32_t>(std::strlen(msg)));
  push_str(cmd, msg);

  /* 4. Commit(true) */
  push<uint8_t>(cmd, NVE_OP_COMMIT);
  push<uint8_t>(cmd, 1);

  NveFrameStats stats{};
  const int32_t rc = nve_fabric_host_submit_frame(
      host, NveBufferSlice{.ptr = cmd.data(), .len = cmd.size()}, &stats);
  assert(rc == 0);
  assert(stats.op_count == 4);
  assert(stats.mutation_count == 3);
  assert(nve_fabric_host_node_count(host) == 3);

  NveBufferSlice events = nve_fabric_host_drain_events(host);
  assert(events.len == 4); /* count == 0 */

  nve_fabric_host_destroy(host);
  std::cout << "nve_fabric_bridge_test passed (decode_us=" << stats.decode_us << ")\n";
  return 0;
}
