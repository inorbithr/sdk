import XCTest

@testable import Hello

final class HelloTests: XCTestCase {
    func testHello() {
        XCTAssertEqual(hello(), "hello from @@NAME@@")
    }
}
