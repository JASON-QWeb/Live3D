import Foundation

// Offline replay of two already-masked records from the uploader's console log.
// https://github.com/saermart/DouyinLiveWebFetcher/issues/100#issuecomment-2706330441
let name = Part(text: "涂***:", role: .author)
let body = Part(text: "良咔女神盛典", role: .text)
let comment = chatLine([name, body])!
precondition(comment.author == "涂***" && comment.text == body.text)
var chats = RowTracker<Int>()
precondition(chats.observe([0], pane: .chat, now: 0) { _ in comment }.isEmpty)
precondition(chats.observe([0, 1, 2], pane: .chat, now: 1) { _ in comment } == [comment, comment])
precondition(chats.observe([0, 1, 2], pane: .chat, now: 2) { _ in comment }.isEmpty)

let card = [
    Part(text: "吴***", role: .author), Part(text: "送出了", role: .text),
    Part(text: "小心心", role: .text), Part(text: "gift-icon", role: .giftIcon),
]
let quantity = [Part(text: "x", role: .text), Part(text: "3", role: .text)]
let partial = giftLine(card)!
let complete = giftLine(card + quantity)!
precondition(partial.kind == "unreadable")
precondition(complete.kind == "gift" && complete.author == "吴***")
precondition(complete.gift_name == "小心心" && complete.gift_count == 3)
precondition(giftLine(Array(card.dropFirst()) + quantity)?.kind == "unreadable")
precondition(giftLine(Array(card.dropLast()) + quantity) == complete)

var gifts = RowTracker<Int>()
precondition(gifts.observe([], pane: .gift, now: 0) { _ in partial }.isEmpty)
precondition(gifts.observe([1], pane: .gift, now: 1) { _ in partial }.isEmpty)
precondition(gifts.observe([1], pane: .gift, now: 3) { _ in partial } == [partial])
precondition(gifts.observe([1], pane: .gift, now: 4) { _ in complete }.isEmpty)
precondition(gifts.observe([1], pane: .gift, now: 6) { _ in complete } == [complete])
precondition(gifts.observe([1], pane: .gift, now: 9) { _ in complete }.isEmpty)
