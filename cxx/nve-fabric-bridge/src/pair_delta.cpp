#include "pair_internal.hpp"

#include <algorithm>
#include <array>
#include <cstring>
#include <limits>
#include <optional>
#include <unordered_map>
#include <utility>
#include <vector>

namespace nve {

namespace {

using namespace pair_detail;

struct RawCmdHeader {
  uint16_t flags{0};
  uint32_t surface_id{0};
  uint64_t frame_id{0};
  uint32_t op_count{0};
};

bool scan_nve_command_slices(
    std::span<const uint8_t> bytes,
    RawCmdHeader& out_hdr,
    std::vector<std::pair<size_t, size_t>>& out_ranges) {
  if (bytes.size() < kNveCmdHeaderSize) {
    return false;
  }
  if (read_u32_le(bytes.data()) != NVE_ABI_MAGIC) {
    return false;
  }
  if (read_u16_le(bytes.data() + 4) != NVE_ABI_VERSION) {
    return false;
  }
  out_hdr.flags = read_u16_le(bytes.data() + 6);
  out_hdr.surface_id = read_u32_le(bytes.data() + 8);
  out_hdr.frame_id = read_u64_le(bytes.data() + 12);
  out_hdr.op_count = read_u32_le(bytes.data() + 20);

  size_t pos = kNveCmdHeaderSize;
  out_ranges.clear();
  out_ranges.reserve(std::min<size_t>(static_cast<size_t>(out_hdr.op_count), 4096u));

  auto take = [&](size_t n, const uint8_t*& ptr) -> bool {
    if (n > bytes.size() - pos) {
      return false;
    }
    ptr = bytes.data() + pos;
    pos += n;
    return true;
  };
  auto take_u16 = [&](uint16_t& v) -> bool {
    const uint8_t* p = nullptr;
    if (!take(2u, p)) return false;
    v = read_u16_le(p);
    return true;
  };
  auto take_u32 = [&](uint32_t& v) -> bool {
    const uint8_t* p = nullptr;
    if (!take(4u, p)) return false;
    v = read_u32_le(p);
    return true;
  };

  for (uint32_t idx = 0u; idx < out_hdr.op_count; ++idx) {
    const size_t start = pos;
    const uint8_t* op_ptr = nullptr;
    if (!take(1u, op_ptr)) {
      return false;
    }
    const uint8_t op = *op_ptr;
    const uint8_t* sink = nullptr;
    switch (op) {
      case 1u: {
        uint16_t name_len = 0u;
        if (!take(6u, sink) || !take_u16(name_len) || !take(name_len, sink)) return false;
        break;
      }
      case 2u: {
        if (!take(4u, sink)) return false;
        break;
      }
      case 3u: {
        uint32_t count = 0u;
        if (!take(4u, sink) || !take_u32(count)) return false;
        if (static_cast<size_t>(count) > (bytes.size() - pos) / 4u) return false;
        if (!take(static_cast<size_t>(count) * 4u, sink)) return false;
        break;
      }
      case 4u: {
        uint32_t len = 0u;
        if (!take(4u, sink) || !take_u32(len) || !take(static_cast<size_t>(len), sink)) {
          return false;
        }
        break;
      }
      case 5u: {
        if (!take(81u, sink)) return false;
        break;
      }
      case 6u: {
        if (!take(36u, sink)) return false;
        break;
      }
      case 7u: {
        uint16_t key_len = 0u;
        const uint8_t* vtag_ptr = nullptr;
        if (!take(4u, sink) || !take_u16(key_len) || !take(key_len, sink) || !take(1u, vtag_ptr)) {
          return false;
        }
        switch (*vtag_ptr) {
          case 0u:
            break;
          case 1u:
            if (!take(1u, sink)) return false;
            break;
          case 2u:
          case 3u:
            if (!take(8u, sink)) return false;
            break;
          case 4u:
          case 5u: {
            uint32_t vlen = 0u;
            if (!take_u32(vlen) || !take(static_cast<size_t>(vlen), sink)) return false;
            break;
          }
          default:
            return false;
        }
        break;
      }
      case 8u: {
        if (!take(12u, sink)) return false;
        break;
      }
      case 9u: {
        uint16_t nlen = 0u;
        uint32_t plen = 0u;
        if (!take(4u, sink) || !take_u16(nlen) || !take(nlen, sink) || !take_u32(plen) ||
            !take(static_cast<size_t>(plen), sink)) {
          return false;
        }
        break;
      }
      case 10u: {
        if (!take(1u, sink)) return false;
        break;
      }
      default:
        return false;
    }
    out_ranges.emplace_back(start, pos);
  }
  return pos == bytes.size();
}

bool raw_op_identity(std::span<const uint8_t> op_slice, uint64_t& out_key) noexcept {
  if (op_slice.empty()) {
    return false;
  }
  const uint8_t opcode = op_slice[0];
  if (opcode == 10u) {
    out_key = (static_cast<uint64_t>(opcode) << 32u);
    return true;
  }
  if (op_slice.size() < 5u) {
    return false;
  }
  const uint32_t node_id = read_u32_le(op_slice.data() + 1);
  out_key = (static_cast<uint64_t>(opcode) << 32u) | static_cast<uint64_t>(node_id);
  return true;
}

} // namespace

namespace pair_detail {

void encode_xor_rle_delta_into(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next,
    std::vector<uint8_t>& out) {
  out.clear();
  const size_t len = next.size();
  const size_t min_reserve =
      std::max<size_t>(64u, std::min<size_t>(len + (len >> 6u) + 16u, kMaxDecodedFrameBytes));
  if (out.capacity() < min_reserve) {
    out.reserve(min_reserve);
  }
  write_varint(static_cast<uint64_t>(len), out);

  size_t i = 0u;
  auto base_at = [&](size_t idx) noexcept -> uint8_t {
    return idx < base.size() ? base[idx] : 0u;
  };

  while (i < len) {
    const size_t zero_start = i;
    while (i < len && static_cast<uint8_t>(base_at(i) ^ next[i]) == 0u) {
      ++i;
    }
    const size_t zero_run = i - zero_start;
    if (zero_run > 0u) {
      if (zero_run <= 127u) {
        out.push_back(static_cast<uint8_t>(zero_run));
      } else {
        out.push_back(0x00u);
        write_varint(static_cast<uint64_t>(zero_run), out);
      }
    }
    if (i >= len) {
      break;
    }

    const size_t lit_start = i;
    while (i < len) {
      if (static_cast<uint8_t>(base_at(i) ^ next[i]) == 0u) {
        const bool next_also_zero =
            (i + 1u < len) && (static_cast<uint8_t>(base_at(i + 1u) ^ next[i + 1u]) == 0u);
        if (next_also_zero || i + 1u == len) {
          break;
        }
      }
      ++i;
    }
    const size_t lit_len = i - lit_start;
    if (lit_len <= 127u) {
      out.push_back(static_cast<uint8_t>(0x80u | lit_len));
    } else {
      out.push_back(0x80u);
      write_varint(static_cast<uint64_t>(lit_len), out);
    }
    const size_t out_offset = out.size();
    out.resize(out_offset + lit_len);
    uint8_t* dst = out.data() + out_offset;
    for (size_t k = 0u; k < lit_len; ++k) {
      dst[k] = static_cast<uint8_t>(base_at(lit_start + k) ^ next[lit_start + k]);
    }
  }
}

} // namespace pair_detail

namespace pair_wire {

using namespace pair_detail;

std::vector<uint8_t> encode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next) {
  std::vector<uint8_t> out;
  encode_xor_rle_delta_into(base, next, out);
  return out;
}

bool decode_xor_rle_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out) {
  size_t pos = 0u;
  uint64_t target_len_u64 = 0u;
  if (!read_varint(delta_payload, pos, target_len_u64)) {
    return false;
  }
  if (target_len_u64 > static_cast<uint64_t>(kMaxDecodedFrameBytes)) {
    return false;
  }
  const size_t target_len = static_cast<size_t>(target_len_u64);
  out.clear();
  out.reserve(target_len);

  while (out.size() < target_len) {
    if (pos >= delta_payload.size()) {
      return false;
    }
    const uint8_t tag = delta_payload[pos++];
    if ((tag & 0x80u) == 0u) {
      uint64_t run_len_u64 = 0u;
      if (tag == 0u) {
        if (!read_varint(delta_payload, pos, run_len_u64)) {
          return false;
        }
      } else {
        run_len_u64 = static_cast<uint64_t>(tag);
      }
      if (run_len_u64 == 0u || run_len_u64 > static_cast<uint64_t>(target_len - out.size())) {
        return false;
      }
      const size_t run_len = static_cast<size_t>(run_len_u64);
      const size_t start = out.size();
      const size_t end = start + run_len;
      const size_t from_base_end = std::min(end, base.size());
      if (start < from_base_end) {
        out.insert(
            out.end(),
            base.begin() + static_cast<ptrdiff_t>(start),
            base.begin() + static_cast<ptrdiff_t>(from_base_end));
      }
      const size_t pad_start = std::max(from_base_end, start);
      if (end > pad_start) {
        out.resize(out.size() + (end - pad_start), 0u);
      }
    } else {
      uint64_t lit_len_u64 = 0u;
      if (tag == 0x80u) {
        if (!read_varint(delta_payload, pos, lit_len_u64)) {
          return false;
        }
      } else {
        lit_len_u64 = static_cast<uint64_t>(tag & 0x7Fu);
      }
      if (lit_len_u64 == 0u || lit_len_u64 > static_cast<uint64_t>(target_len - out.size())) {
        return false;
      }
      const size_t lit_len = static_cast<size_t>(lit_len_u64);
      if (delta_payload.size() - pos < lit_len) {
        return false;
      }
      const size_t base_offset = out.size();
      const uint8_t* xor_bytes = delta_payload.data() + pos;
      pos += lit_len;
      out.resize(base_offset + lit_len);
      uint8_t* dst = out.data() + base_offset;
      const size_t base_avail =
          (base_offset < base.size()) ? std::min(lit_len, base.size() - base_offset) : 0u;
      for (size_t k = 0u; k < base_avail; ++k) {
        dst[k] = static_cast<uint8_t>(base[base_offset + k] ^ xor_bytes[k]);
      }
      if (lit_len > base_avail) {
        std::memcpy(dst + base_avail, xor_bytes + base_avail, lit_len - base_avail);
      }
    }
  }
  return pos == delta_payload.size();
}

std::optional<std::vector<uint8_t>> encode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> next) {
  RawCmdHeader prev_hdr{};
  std::vector<std::pair<size_t, size_t>> prev_ranges;
  if (!scan_nve_command_slices(base, prev_hdr, prev_ranges)) {
    return std::nullopt;
  }
  RawCmdHeader next_hdr{};
  std::vector<std::pair<size_t, size_t>> next_ranges;
  if (!scan_nve_command_slices(next, next_hdr, next_ranges)) {
    return std::nullopt;
  }

  std::unordered_map<uint64_t, std::vector<size_t>> prev_exact;
  prev_exact.reserve(prev_ranges.size());
  std::unordered_map<uint64_t, size_t> prev_by_node;
  prev_by_node.reserve(prev_ranges.size());

  for (size_t idx = 0u; idx < prev_ranges.size(); ++idx) {
    const auto [s, e] = prev_ranges[idx];
    const auto slice = base.subspan(s, e - s);
    const uint64_t h = fnv1a_64(slice);
    auto& vec = prev_exact[h];
    if (vec.size() < 4u) {
      vec.push_back(idx);
    }
    uint64_t key = 0u;
    if (raw_op_identity(slice, key)) {
      prev_by_node[key] = idx;
    }
  }

  std::vector<uint8_t> out;
  out.reserve(std::max<size_t>(64u, std::min<size_t>(next.size() / 2u + 64u, kMaxDecodedFrameBytes)));
  push_u16_le(out, next_hdr.flags);
  write_varint(static_cast<uint64_t>(next_hdr.surface_id), out);
  write_varint(next_hdr.frame_id, out);
  write_varint(static_cast<uint64_t>(next_hdr.op_count), out);

  std::vector<uint8_t> scratch_xor;
  std::vector<uint8_t> scratch_token;

  auto slices_equal = [](std::span<const uint8_t> a, std::span<const uint8_t> b) noexcept -> bool {
    return a.size() == b.size() && (a.empty() || std::memcmp(a.data(), b.data(), a.size()) == 0);
  };

  size_t ni = 0u;
  size_t expected_prev = 0u;
  const size_t n_next = next_ranges.size();
  const size_t n_prev = prev_ranges.size();

  while (ni < n_next) {
    const auto [ns, ne] = next_ranges[ni];
    const auto nslice = next.subspan(ns, ne - ns);

    std::optional<size_t> matched_prev;
    if (expected_prev < n_prev) {
      const auto [ps, pe] = prev_ranges[expected_prev];
      if (slices_equal(base.subspan(ps, pe - ps), nslice)) {
        matched_prev = expected_prev;
      }
    }
    if (!matched_prev.has_value()) {
      const auto it = prev_exact.find(fnv1a_64(nslice));
      if (it != prev_exact.end()) {
        for (size_t pidx : it->second) {
          const auto [ps, pe] = prev_ranges[pidx];
          if (slices_equal(base.subspan(ps, pe - ps), nslice)) {
            matched_prev = pidx;
            break;
          }
        }
      }
    }

    if (matched_prev.has_value()) {
      const size_t pstart = *matched_prev;
      size_t run = 1u;
      while (ni + run < n_next && pstart + run < n_prev) {
        const auto [ns2, ne2] = next_ranges[ni + run];
        const auto [ps2, pe2] = prev_ranges[pstart + run];
        if (!slices_equal(next.subspan(ns2, ne2 - ns2), base.subspan(ps2, pe2 - ps2))) {
          break;
        }
        ++run;
      }
      out.push_back(kCmdDeltaCopyRun);
      write_varint(static_cast<uint64_t>(pstart), out);
      write_varint(static_cast<uint64_t>(run), out);
      ni += run;
      expected_prev = pstart + run;
    } else {
      bool emitted_xor = false;
      uint64_t key = 0u;
      if (raw_op_identity(nslice, key)) {
        const auto it = prev_by_node.find(key);
        if (it != prev_by_node.end()) {
          const size_t pidx = it->second;
          const auto [ps, pe] = prev_ranges[pidx];
          encode_xor_rle_delta_into(base.subspan(ps, pe - ps), nslice, scratch_xor);
          scratch_token.clear();
          scratch_token.push_back(kCmdDeltaXorOp);
          write_varint(static_cast<uint64_t>(pidx), scratch_token);
          write_varint(static_cast<uint64_t>(scratch_xor.size()), scratch_token);
          scratch_token.insert(scratch_token.end(), scratch_xor.begin(), scratch_xor.end());

          const size_t lit_overhead = 1u + (nslice.size() < 128u ? 1u : 2u) + nslice.size();
          if (scratch_token.size() < lit_overhead) {
            out.insert(out.end(), scratch_token.begin(), scratch_token.end());
            emitted_xor = true;
            expected_prev = pidx + 1u;
          }
        }
      }
      if (!emitted_xor) {
        out.push_back(kCmdDeltaLiteralOp);
        write_varint(static_cast<uint64_t>(nslice.size()), out);
        out.insert(out.end(), nslice.begin(), nslice.end());
        if (expected_prev < std::numeric_limits<size_t>::max()) {
          ++expected_prev;
        }
      }
      ++ni;
    }
  }
  return out;
}

bool decode_command_op_delta(
    std::span<const uint8_t> base,
    std::span<const uint8_t> delta_payload,
    std::vector<uint8_t>& out) {
  RawCmdHeader prev_hdr{};
  std::vector<std::pair<size_t, size_t>> prev_ranges;
  if (!scan_nve_command_slices(base, prev_hdr, prev_ranges)) {
    return false;
  }

  size_t pos = 0u;
  if (delta_payload.size() < 2u) {
    return false;
  }
  const uint16_t flags = read_u16_le(delta_payload.data());
  pos += 2u;

  uint64_t surface_id_u64 = 0u;
  uint64_t frame_id = 0u;
  uint64_t op_count_u64 = 0u;
  if (!read_varint(delta_payload, pos, surface_id_u64) ||
      !read_varint(delta_payload, pos, frame_id) ||
      !read_varint(delta_payload, pos, op_count_u64)) {
    return false;
  }
  if (surface_id_u64 > std::numeric_limits<uint32_t>::max() ||
      op_count_u64 > std::numeric_limits<uint32_t>::max()) {
    return false;
  }
  const uint32_t surface_id = static_cast<uint32_t>(surface_id_u64);
  const uint32_t op_count = static_cast<uint32_t>(op_count_u64);

  out.clear();
  out.reserve(std::max<size_t>(std::min<size_t>(base.size(), 64u * 1024u), kNveCmdHeaderSize));
  push_u32_le(out, NVE_ABI_MAGIC);
  push_u16_le(out, NVE_ABI_VERSION);
  push_u16_le(out, flags);
  push_u32_le(out, surface_id);
  push_u64_le(out, frame_id);
  push_u32_le(out, op_count);

  std::vector<uint8_t> xor_scratch;
  uint32_t emitted_ops = 0u;

  while (pos < delta_payload.size()) {
    const uint8_t tag = delta_payload[pos++];
    switch (tag) {
      case kCmdDeltaCopyRun: {
        uint64_t start_u64 = 0u;
        uint64_t count_u64 = 0u;
        if (!read_varint(delta_payload, pos, start_u64) ||
            !read_varint(delta_payload, pos, count_u64)) {
          return false;
        }
        if (count_u64 == 0u || start_u64 > prev_ranges.size() ||
            count_u64 > prev_ranges.size() - start_u64) {
          return false;
        }
        const size_t start_idx = static_cast<size_t>(start_u64);
        const size_t count = static_cast<size_t>(count_u64);
        for (size_t i = 0u; i < count; ++i) {
          const auto [ps, pe] = prev_ranges[start_idx + i];
          const size_t slice_len = pe - ps;
          if (slice_len > kMaxDecodedFrameBytes - out.size()) {
            return false;
          }
          out.insert(
              out.end(),
              base.begin() + static_cast<ptrdiff_t>(ps),
              base.begin() + static_cast<ptrdiff_t>(pe));
          if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
            ++emitted_ops;
          }
        }
        break;
      }
      case kCmdDeltaLiteralOp: {
        uint64_t len_u64 = 0u;
        if (!read_varint(delta_payload, pos, len_u64)) {
          return false;
        }
        if (len_u64 == 0u || len_u64 > static_cast<uint64_t>(delta_payload.size() - pos)) {
          return false;
        }
        const size_t len = static_cast<size_t>(len_u64);
        if (len > kMaxDecodedFrameBytes - out.size()) {
          return false;
        }
        out.insert(
            out.end(),
            delta_payload.begin() + static_cast<ptrdiff_t>(pos),
            delta_payload.begin() + static_cast<ptrdiff_t>(pos + len));
        pos += len;
        if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
          ++emitted_ops;
        }
        break;
      }
      case kCmdDeltaXorOp: {
        uint64_t prev_idx_u64 = 0u;
        uint64_t xor_len_u64 = 0u;
        if (!read_varint(delta_payload, pos, prev_idx_u64) ||
            prev_idx_u64 >= prev_ranges.size() ||
            !read_varint(delta_payload, pos, xor_len_u64)) {
          return false;
        }
        if (xor_len_u64 == 0u || xor_len_u64 > static_cast<uint64_t>(delta_payload.size() - pos)) {
          return false;
        }
        const auto [ps, pe] = prev_ranges[static_cast<size_t>(prev_idx_u64)];
        const size_t xor_len = static_cast<size_t>(xor_len_u64);
        if (!decode_xor_rle_delta(
                base.subspan(ps, pe - ps), delta_payload.subspan(pos, xor_len), xor_scratch)) {
          return false;
        }
        pos += xor_len;
        if (xor_scratch.size() > kMaxDecodedFrameBytes - out.size()) {
          return false;
        }
        out.insert(out.end(), xor_scratch.begin(), xor_scratch.end());
        if (emitted_ops < std::numeric_limits<uint32_t>::max()) {
          ++emitted_ops;
        }
        break;
      }
      default:
        return false;
    }
  }
  return emitted_ops == op_count;
}

} // namespace pair_wire

} // namespace nve
