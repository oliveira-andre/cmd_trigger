import XCTest

/// The iPhone app against a real CmdTrigger agent and server on this Mac (the simulator shares
/// its network). `ios/ui-test.sh` starts them and passes these through xcodebuild:
///
/// - `CT_NAME`: the computer's name; `CT_CODE`: a pairing code it is showing
/// - `CT_SERVER`, `CT_EMAIL`, `CT_PASSWORD`: the server account
/// - `CT_SHOTS`: where to save screenshots
///
/// The tests run in order, with the harness changing things on the Mac in between.
final class CmdTriggerUITests: XCTestCase {
    private let env = ProcessInfo.processInfo.environment
    private var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(reset: Bool) {
        app = XCUIApplication()
        if reset { app.launchEnvironment["CT_RESET"] = "1" }
        app.launch()
    }

    private func shot(_ file: String) {
        guard let dir = env["CT_SHOTS"] else { return }
        let data = XCUIScreen.main.screenshot().pngRepresentation
        try? data.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(file).png"))
    }

    private var computerName: String { env["CT_NAME"] ?? "Live Mac" }

    private func tile(_ label: String) -> XCUIElement { app.buttons["tile.\(label)"] }

    /// Waits for a toast containing `text`.
    @discardableResult
    private func toast(_ text: String, timeout: TimeInterval = 20) -> String {
        let toast = app.descendants(matching: .any)["toast"]
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if toast.exists, toast.label.localizedCaseInsensitiveContains(text) { return toast.label }
            usleep(200_000)
        }
        XCTFail("no toast with “\(text)”; last: \(toast.exists ? toast.label : "none")")
        return ""
    }

    private func typeInto(_ element: XCUIElement, _ text: String) {
        XCTAssertTrue(element.waitForExistence(timeout: 10), "\(element)")
        element.tap()
        element.typeText(text)
    }

    /// Pair over the Wi-Fi (found by Bonjour), run commands and routines, see the activity,
    /// sign in to the server.
    func test1PairAndRun() throws {
        let code = try XCTUnwrap(env["CT_CODE"], "CT_CODE")
        launch(reset: true)
        XCTAssertTrue(app.buttons["home.pair"].waitForExistence(timeout: 10))
        shot("01-welcome")

        app.buttons["home.pair"].tap()
        let found = app.buttons["pair.found.\(computerName)"]
        XCTAssertTrue(found.waitForExistence(timeout: 15), "the computer shows up on the Wi-Fi (Bonjour)")
        shot("02-pair-found")
        found.tap()
        typeInto(app.textFields["pair.code"], code)
        shot("03-pair-code")
        app.buttons["pair.submit"].tap()
        toast("Paired with \(computerName)")

        XCTAssertTrue(tile("Marker").waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Home Wi-Fi"].waitForExistence(timeout: 10))
        shot("04-home")

        // A failing command: the toast says so and its output opens.
        tile("Fails").tap()
        toast("Fails: failed (exit 5)")
        XCTAssertTrue(app.descendants(matching: .any)["run.output"].waitForExistence(timeout: 5))
        shot("05-failed-output")
        app.buttons["Close"].tap()

        // Parameters.
        tile("Marker").tap()
        typeInto(app.textFields["ask.params"], "from-iphone")
        shot("06-params")
        app.buttons["ask.run"].tap()
        toast("Marker: done")

        // On/off.
        tile("Lights").tap()
        XCTAssertTrue(app.buttons["ask.off"].waitForExistence(timeout: 5))
        app.buttons["ask.off"].tap()
        toast("Lights: done")

        // A routine (automation).
        app.swipeUp()
        tile("Wake test").tap()
        toast("Wake test: done")

        // The screen of an awake computer.
        app.swipeDown()
        app.buttons["wake.\(computerName)"].tap()
        toast("screen on")
        shot("07-after-runs")

        app.tabBars.buttons["Activity"].tap()
        XCTAssertTrue(app.staticTexts["Marker"].firstMatch.waitForExistence(timeout: 10))
        shot("08-activity")

        app.tabBars.buttons["Settings"].tap()
        typeInto(app.textFields["server.url"], env["CT_SERVER"] ?? "")
        typeInto(app.textFields["server.email"], env["CT_EMAIL"] ?? "")
        typeInto(app.secureTextFields["server.password"], env["CT_PASSWORD"] ?? "")
        app.buttons["server.signin"].tap()
        XCTAssertTrue(app.buttons["Sign out"].waitForExistence(timeout: 20), "signed in to the server")
        shot("09-settings")
        app.tabBars.buttons["Home"].tap()
        shot("10-home-final")
    }

    /// The app on the computer has quit; its background service is still connected to the
    /// server. Its commands run through the server, the app's own are greyed out.
    func test2ThroughServer() {
        launch(reset: false)
        XCTAssertTrue(app.staticTexts["Via server"].waitForExistence(timeout: 30))
        XCTAssertTrue(tile("Boot").waitForExistence(timeout: 5))
        XCTAssertTrue(tile("Boot").isEnabled)
        XCTAssertFalse(tile("Marker").isEnabled, "the app's own commands need the app")
        shot("11-via-server")
        tile("Boot").tap()
        toast("Boot: done", timeout: 40)
        shot("12-ran-via-server")
    }

    /// Wake: the computer doesn't answer; the phone keeps sending wake-up packets until it does
    /// (the harness starts the app again 6 s after the tap, signalled by a `wake-tapped` file).
    func test3Wake() {
        launch(reset: false)
        let wake = app.buttons["wake.\(computerName)"]
        XCTAssertTrue(wake.waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["Via server"].waitForExistence(timeout: 30))
        wake.tap()
        let waking = expectation(for: NSPredicate(format: "label CONTAINS 'Waking'"), evaluatedWith: wake)
        wait(for: [waking], timeout: 5)
        shot("13-waking")
        // Tell the harness to bring the computer back now.
        if let dir = env["CT_SHOTS"] { try? Data().write(to: URL(fileURLWithPath: dir).appendingPathComponent("wake-tapped")) }
        toast("is awake", timeout: 50)
        XCTAssertTrue(app.staticTexts["Home Wi-Fi"].waitForExistence(timeout: 10))
        shot("14-awake")
    }
}
