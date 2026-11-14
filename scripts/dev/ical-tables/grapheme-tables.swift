// Generates the grapheme-cluster-break property ranges that Swift's String.count observes.
import Foundation
func ranges(_ pred: (Unicode.Scalar) -> Bool) -> [(UInt32, UInt32)] {
  var out: [(UInt32, UInt32)] = []
  var cur: (UInt32, UInt32)? = nil
  for v in UInt32(0)...0x10FFFF {
    guard let s = Unicode.Scalar(v) else { continue }
    if pred(s) {
      if let c = cur, c.1 + 1 == v { cur = (c.0, v) } else { if let c = cur { out.append(c) }; cur = (v, v) }
    } else if let c = cur, c.1 + 1 < v && Unicode.Scalar(c.1 + 1) != nil { out.append(c); cur = nil }
  }
  if let c = cur { out.append(c) }
  return out
}
let a = "a"
let extend = ranges { (a + String($0)).count == 1 }
let extendSet = Set(extend.flatMap { r in (r.0...r.1).map { $0 } })
let prepend = ranges { (String($0) + a).count == 1 }
let control = ranges { (String($0) + "\u{0301}").count == 2 }
let indicConsonant = ranges { !extendSet.contains($0.value) && (String($0) + "\u{094D}" + String($0)).count == 1 }
let extpict = ranges { !extendSet.contains($0.value) && (String($0) + "\u{200D}" + String($0)).count == 1 }
func dump(_ name: String, _ r: [(UInt32, UInt32)]) {
  print("pub(crate) const \(name): &[(u32, u32)] = &[")
  var line = "   "
  for x in r {
    let item = " (0x\(String(x.0, radix: 16)), 0x\(String(x.1, radix: 16))),"
    if line.count + item.count > 118 { print(line); line = "   " }
    line += item
  }
  print(line)
  print("];")
}
dump("EXTEND_LIKE", extend); dump("PREPEND", prepend); dump("CONTROL", control); dump("EXTENDED_PICTOGRAPHIC", extpict); dump("INDIC_CONSONANT", indicConsonant)
