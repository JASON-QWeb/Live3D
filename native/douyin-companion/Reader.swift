import AppKit
import ApplicationServices

private func emit(_ line: CaptureLine) {
    guard var data = try? JSONEncoder().encode(line) else { return }
    data.append(0x0a)
    FileHandle.standardOutput.write(data)
}

final class CompanionReader {
    private let findApplication: () -> NSRunningApplication?

    init(findApplication: @escaping () -> NSRunningApplication?) {
        self.findApplication = findApplication
    }

    private var app: NSRunningApplication?
    private var root: AXUIElement?
    private var requestedTree = false
    private var panels: Panels?
    private var notifications: PanelNotifications?
    private var chat = RowTracker<AXUIElement>()
    private var gifts = RowTracker<AXUIElement>()
    private var lastStatus = ""
    private var nextDiscovery: TimeInterval = 0
    private var scheduled = false

    private func status(_ message: String) {
        guard message != lastStatus else { return }
        lastStatus = message
        emit(CaptureLine(kind: "status", text: message))
    }

    private func resetPanels() {
        notifications = nil
        panels = nil
        chat = RowTracker()
        gifts = RowTracker()
        nextDiscovery = 0
    }

    func detach() {
        resetPanels()
        if requestedTree, let root {
            AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanFalse)
        }
        requestedTree = false
        root = nil
        app = nil
    }

    private func attach() -> Bool {
        if let app, !app.isTerminated { return true }
        detach()
        guard let app = findApplication() else {
            status("等待打开抖音直播伴侣")
            return false
        }
        self.app = app
        let root = AXUIElementCreateApplication(app.processIdentifier)
        self.root = root
        AXUIElementSetMessagingTimeout(root, 0.5)
        return true
    }

    private func scheduleRead() {
        guard !scheduled else { return }
        scheduled = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            self?.scheduled = false
            self?.tick()
        }
    }

    func tick() {
        guard AXIsProcessTrusted() else {
            if app != nil { detach() }
            status("等待辅助功能授权，请在系统设置的「隐私与安全性 → 辅助功能」中允许 Avatar3D")
            return
        }
        guard attach(), let root, let app else { return }
        let now = ProcessInfo.processInfo.systemUptime
        if panels == nil {
            guard now >= nextDiscovery else { return }
            nextDiscovery = now + 1.5
            // Electron can register this attribute after its process appears.
            if attribute(root, "AXManualAccessibility") as? Bool == false {
                requestedTree = AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString,
                                                             kCFBooleanTrue) == .success
            }
            guard let found = Panels.find(in: root) else {
                status("等待伴侣显示「互动消息」面板")
                return
            }
            panels = found
            notifications = PanelNotifications(pid: app.processIdentifier, panels: found) { [weak self] in
                self?.scheduleRead()
            }
        }
        guard let panels, let chatRows = panels.rows(in: .chat), let giftRows = panels.rows(in: .gift) else {
            resetPanels()
            status("正在重新定位互动消息面板")
            return
        }
        status("已连接伴侣，正在读取评论和礼物")
        chat.observe(chatRows, pane: .chat, now: now) { chatLine(visibleParts(of: $0)) }.forEach(emit)
        gifts.observe(giftRows, pane: .gift, now: now) { giftLine(visibleParts(of: $0)) }.forEach(emit)
    }
}

func runReader(findApplication: @escaping () -> NSRunningApplication?) {
    let reader = CompanionReader(findApplication: findApplication)
    if CommandLine.arguments.contains("--request-access") && !AXIsProcessTrusted() {
        let option = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
        _ = AXIsProcessTrustedWithOptions([option: true] as CFDictionary)
    }
    func finish() {
        reader.detach()
        exit(EXIT_SUCCESS)
    }
    let stopSignals = [SIGTERM, SIGINT].map { number -> DispatchSourceSignal in
        signal(number, SIG_IGN)
        let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
        source.setEventHandler { finish() }
        source.resume()
        return source
    }
    // The control room owns the write end. EOF also covers a parent process crash.
    FileHandle.standardInput.readabilityHandler = { input in
        if input.availableData.isEmpty {
            input.readabilityHandler = nil
            DispatchQueue.main.async { finish() }
        }
    }
    reader.tick()
    let timer = Timer(timeInterval: 0.8, repeats: true) { _ in reader.tick() }
    RunLoop.main.add(timer, forMode: .common)
    withExtendedLifetime(stopSignals) { RunLoop.main.run() }
}
