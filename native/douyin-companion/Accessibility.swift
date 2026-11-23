import ApplicationServices
import Foundation

func attribute(_ node: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(node, key as CFString, &value) == .success else { return nil }
    return value
}

private func children(_ node: AXUIElement) -> [AXUIElement] {
    attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? []
}

private func classes(_ node: AXUIElement) -> Set<String> {
    Set(attribute(node, "AXDOMClassList") as? [String] ?? [])
}

struct Panels {
    let gift: AXUIElement
    let chat: AXUIElement

    static func find(in app: AXUIElement) -> Panels? {
        var stack = attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []
        while let node = stack.popLast() {
            let nodes = children(node)
            // These two containers stay in the panel when the empty labels
            // are replaced by messages, including in the detached window.
            if nodes.count >= 2 {
                var gift: AXUIElement?
                var chat: AXUIElement?
                for child in nodes {
                    let css = classes(child)
                    if css.isSuperset(of: ["relative", "w-full", "h-full", "overflow-hidden"]) {
                        gift = child
                    } else if css == ["relative", "min-h-0", "flex-1"] {
                        chat = child
                    }
                }
                if let gift, let chat { return Panels(gift: gift, chat: chat) }
            }
            stack.append(contentsOf: nodes)
        }
        return nil
    }

    func rows(in pane: Pane) -> [AXUIElement]? {
        let container = pane == .chat ? chat : gift
        guard let nodes = attribute(container, kAXChildrenAttribute) as? [AXUIElement] else { return nil }
        var stack = Array(nodes.reversed())
        var rows = [AXUIElement]()
        while let node = stack.popLast() {
            let css = classes(node)
            let wrapper = css.contains("relative") && css.contains("w-full") && !css.contains("h-full")
            let card = pane == .chat ? css.isSuperset(of: ["break-words", "whitespace-pre-wrap"])
                                    : css.isSuperset(of: ["inline-flex", "flex-wrap", "gap-x-[6px]"])
            if wrapper || card {
                rows.append(node)
            } else {
                stack.append(contentsOf: children(node).reversed())
            }
        }
        return rows
    }
}

func visibleParts(of row: AXUIElement) -> [Part] {
    var result = [Part]()
    var stack: [(AXUIElement, PartRole)] = [(row, .text)]
    while let (node, inherited) = stack.popLast() {
        let role = attribute(node, kAXRoleAttribute) as? String
        let css = classes(node)
        let partRole: PartRole
        if css.contains("text-[#ffd4f5]") {
            partRole = .giftName
        } else if role == "AXLink" || css.contains("cursor-default") && css.contains("text-[#8ce7ff]") {
            partRole = .author
        } else {
            partRole = inherited
        }
        let nodes = children(node)
        if !nodes.isEmpty {
            stack.append(contentsOf: nodes.reversed().map { ($0, partRole) })
            continue
        }
        let label = [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute]
            .lazy.compactMap { attribute(node, $0) as? String }.first { !$0.isEmpty }
        guard let label else { continue }
        if role == kAXImageRole {
            if label == "gift-icon" { result.append(Part(text: label, role: .giftIcon)) }
        } else if role == kAXStaticTextRole || role == "AXLink" || partRole != .text {
            result.append(Part(text: label, role: partRole))
        }
    }
    return result
}

final class PanelNotifications {
    private var observer: AXObserver?

    init(pid: pid_t, panels: Panels, changed: @escaping () -> Void) {
        self.changed = changed
        let status = AXObserverCreate(pid, { _, _, _, context in
            guard let context else { return }
            Unmanaged<PanelNotifications>.fromOpaque(context).takeUnretainedValue().changed()
        }, &observer)
        guard status == .success, let observer else { return }
        for node in [panels.gift, panels.chat] {
            for notification in [kAXLayoutChangedNotification, kAXValueChangedNotification,
                                 kAXCreatedNotification, kAXUIElementDestroyedNotification] {
                AXObserverAddNotification(observer, node, notification as CFString,
                                          Unmanaged.passUnretained(self).toOpaque())
            }
        }
        CFRunLoopAddSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
    }

    private let changed: () -> Void

    deinit {
        if let observer {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
        }
    }
}
