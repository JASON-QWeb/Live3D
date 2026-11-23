import Foundation

enum Pane { case chat, gift }
enum PartRole { case text, author, giftName, giftIcon }

struct Part {
    let text: String
    let role: PartRole
    var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }
}

struct CaptureLine: Codable, Equatable {
    let kind: String
    let text: String
    var author: String?
    var gift_name: String?
    var gift_count: UInt32?
}

func chatLine(_ parts: [Part]) -> CaptureLine? {
    // The name and its trailing colon may be separate accessible text nodes.
    let parts = parts.filter { !$0.trimmed.isEmpty && $0.role != .giftIcon }
    guard let split = parts.firstIndex(where: { part in
        part.trimmed.last.map { $0 == ":" || $0 == "：" } ?? false
    }) else { return nil }
    let label = parts[split].trimmed
    let name = label.count > 1 ? String(label.dropLast()) : parts[..<split].last?.trimmed
    let body = parts.dropFirst(split + 1).map(\.text).joined()
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard let author = name?.trimmingCharacters(in: .whitespacesAndNewlines),
          !author.isEmpty, !body.isEmpty else { return nil }
    return CaptureLine(kind: "chat", text: body, author: author)
}

private let quantityPattern = try! NSRegularExpression(
    pattern: #"(?:[×xX]\s*([0-9]+)\s*$|([0-9]+)\s*(?:个|份|朵|支|束|组))"#
)

private func quantity(_ text: String) -> UInt32? {
    let range = NSRange(text.startIndex..., in: text)
    guard let match = quantityPattern.firstMatch(in: text, range: range) else { return nil }
    for index in 1..<match.numberOfRanges {
        if let span = Range(match.range(at: index), in: text),
           let number = UInt32(text[span]), number > 0 { return number }
    }
    return nil
}

private func giftName(in parts: [Part]) -> String? {
    if let named = parts.first(where: { $0.role == .giftName }) { return named.trimmed }
    // Chromium sometimes flattens the gift-name span. The companion renders
    // either the gift icon or a multiplier immediately after the name.
    let marker = parts.firstIndex { $0.role == .giftIcon }
        ?? parts.lastIndex { $0.trimmed.first.map { "xX×".contains($0) } == true }
    guard let marker, marker > 1, parts[marker - 1].role == .text else { return nil }
    var label = parts[marker - 1].trimmed
    guard !label.hasPrefix("送给"), quantity(label) == nil else { return nil }
    if let verb = ["送出了", "送出", "送了", "赠送"].first(where: label.hasPrefix) {
        label = String(label.dropFirst(verb.count)).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    return label.isEmpty ? nil : label
}

func giftLine(_ parts: [Part]) -> CaptureLine? {
    let visible = parts.filter { !$0.trimmed.isEmpty }
    guard !visible.isEmpty else { return nil }
    let words = visible.filter { $0.role != .giftIcon }
    let unknown = CaptureLine(kind: "unreadable", text: words.map(\.trimmed).joined(separator: " "))
    // The sender precedes the recipient, gift and quantity on the card.
    guard let sender = words.first, let gift = giftName(in: visible),
          sender.role == .author || (sender.role == .text && sender.trimmed != gift &&
             !["送给", "送出", "送了", "赠送"].contains(where: sender.trimmed.hasPrefix)) else { return unknown }
    let countText = words.dropFirst().filter { $0.trimmed != gift && $0.role != .author }
        .map(\.trimmed).joined(separator: " ")
    guard let count = quantity(countText) else { return unknown }
    return CaptureLine(kind: "gift", text: "", author: sender.trimmed, gift_name: gift, gift_count: count)
}

// AX element identity distinguishes two identical comments on separate rows.
// A gift card can update its count in place before it settles.
struct RowTracker<Key: Hashable> {
    private var baseline = true
    private var seen = Set<Key>()
    private var order = [Key]()
    private var pending = [Key: (line: CaptureLine, changed: TimeInterval, reported: Bool)]()

    mutating func observe(_ keys: [Key], pane: Pane, now: TimeInterval,
                          read: (Key) -> CaptureLine?) -> [CaptureLine] {
        var output = [CaptureLine]()
        for key in keys {
            let fresh = seen.insert(key).inserted
            if fresh { order.append(key) }
            guard !baseline, fresh || pending[key] != nil, let line = read(key) else { continue }
            if pane == .chat {
                output.append(line)
            } else if pending[key]?.line != line {
                pending[key] = (line, now, false)
            }
        }
        baseline = false
        let visible = Set(keys)
        for key in order {
            if let entry = pending[key], now - entry.changed >= 2 || !visible.contains(key) {
                if !entry.reported { output.append(entry.line) }
                if visible.contains(key) && entry.line.kind == "unreadable" {
                    pending[key]?.reported = true
                } else {
                    pending.removeValue(forKey: key)
                }
            }
        }
        if order.count > 512 {
            order.removeAll { key in
                guard !visible.contains(key) else { return false }
                seen.remove(key)
                return true
            }
        }
        return output
    }
}
