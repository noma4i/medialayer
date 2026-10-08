public protocol MediaSource: Sendable {
    var url: String { get }
    var kind: Kind { get }
    var headers: [String: String] { get }
}
