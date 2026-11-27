#!/usr/bin/env bash
# Compiles each variant sequentially under the watchdog (cap 3 GB / 60 s).
cd "$(dirname "$0")"; mkdir -p out
run() { # name flags proto endexpr
  local name=$1 flags=$2 proto=$3 end=$4 extra=""
  [ "$proto" = Collection ] && extra="    func index(after i: Int) -> Int { i + 1 }"
  cat > out/$name.swift <<SRC
struct HugeBytes: $proto {
    var startIndex: Int { 0 }
    $end
    subscript(_: Int) -> UInt8 { 0 }
$extra
}
SRC
  printf '%-28s %-8s ' "$name" "$flags"
  ./watchdog.sh 3145728 60 -- swiftc $flags -parse-as-library -emit-object out/$name.swift -o out/$name.o 2>&1 | tail -1
}
C='var endIndex: Int {'
run rac_2p32_O        -O     RandomAccessCollection "$C Int(UInt32.max) + 1 }"
run rac_2p32_Osize    -Osize RandomAccessCollection "$C Int(UInt32.max) + 1 }"
run rac_2p32_Onone    -Onone RandomAccessCollection "$C Int(UInt32.max) + 1 }"
run coll_2p32_O       -O     Collection             "$C Int(UInt32.max) + 1 }"
run bidi_2p32_O       -O     BidirectionalCollection "$C Int(UInt32.max) + 1 }
    func index(after i: Int) -> Int { i + 1 }
    func index(before i: Int) -> Int { i - 1 }"
run rac_intmax_O      -O     RandomAccessCollection "$C Int.max }"
run rac_2p10_O        -O     RandomAccessCollection "$C 1 << 10 }"
run rac_2p12_O        -O     RandomAccessCollection "$C 1 << 12 }"
run rac_2p14_O        -O     RandomAccessCollection "$C 1 << 14 }"
run rac_2p16_O        -O     RandomAccessCollection "$C 1 << 16 }"
run rac_2p18_O        -O     RandomAccessCollection "$C 1 << 18 }"
run rac_2p20_O        -O     RandomAccessCollection "$C 1 << 20 }"
run rac_stored_O      -O     RandomAccessCollection "let endIndex: Int = Int(UInt32.max) + 1"
run rac_storedvar_O   -O     RandomAccessCollection "let endIndex: Int"
