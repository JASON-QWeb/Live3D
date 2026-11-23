import AppKit

runReader {
    NSRunningApplication.runningApplications(withBundleIdentifier: "com.bytedance.webcastmate.mac").first
}
