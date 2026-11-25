#pragma once

#import <Foundation/Foundation.h>

#include "nve_abi.generated.h"
#include "nve_rn_component.hpp"
#include "nve_value.hpp"

#include <cstdint>
#include <string_view>

namespace nve::apple {

/// Convert a UTF-8 `std::string_view` to a non-null `NSString` (`@""` when empty or invalid).
NSString * _Nonnull nsstring_from_sv(std::string_view sv);

/// Convert a UTF-8 `std::string_view` to `NSString` (`nil` when empty).
NSString * _Nullable nsstring_or_nil_from_sv(std::string_view sv);

/// Recursively convert an Objective-C Foundation object (`NSDictionary`, `NSArray`, `NSString`,
/// `NSNumber`, `NSData`, `NSNull`) into an arena-backed `NveValue` with zero JSON serialization.
NveValue nve_value_from_objc_object(id _Nullable obj, nve::BumpArena &arena, uint32_t depth = 0u);

/// Recursively convert a zero-copy `BinValueView` into an Objective-C Foundation object.
id _Nonnull objc_object_from_bin_value(const nve::BinValueView &view, uint32_t depth = 0u);

/// Convert a decoded NVE event into an Objective-C payload dictionary.
NSDictionary<NSString *, id> * _Nonnull payload_dictionary_from_event(const nve::DecodedEvent &ev);

} // namespace nve::apple
