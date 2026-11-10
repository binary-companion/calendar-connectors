/// The fixed-seed generator every generated vector uses (never `SystemRandomNumberGenerator`). Each area seeds it
/// with a constant written in that area's file, so adding cases to one area never changes another.
struct LCG {
    var state: UInt64

    mutating func next() -> UInt64 {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return state >> 33
    }

    /// A value in 0..<bound.
    mutating func below(_ bound: Int) -> Int { Int(next() % UInt64(bound)) }

    mutating func int(_ range: ClosedRange<Int>) -> Int { range.lowerBound + below(range.upperBound - range.lowerBound + 1) }

    mutating func pick<T>(_ items: [T]) -> T { items[below(items.count)] }

    /// True `percent` times in 100.
    mutating func chance(_ percent: Int) -> Bool { below(100) < percent }

    /// A random subset of `items` of the given size, in the original order.
    mutating func subset<T>(_ items: [T], count: Int) -> [T] {
        var indexes = Array(items.indices)
        for i in stride(from: indexes.count - 1, to: 0, by: -1) { indexes.swapAt(i, below(i + 1)) }
        return indexes.prefix(count).sorted().map { items[$0] }
    }
}
