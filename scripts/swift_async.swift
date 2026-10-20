private final class UniffiFutureCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: UInt64?
    private let cancelFunc: (UInt64) -> Void

    init(handle: UInt64, cancelFunc: @escaping (UInt64) -> Void) {
        self.handle = handle
        self.cancelFunc = cancelFunc
    }

    func cancel() {
        lock.withLock {
            if let handle { cancelFunc(handle) }
        }
    }

    func finish() {
        lock.withLock { handle = nil }
    }
}

fileprivate func uniffiRustCallAsync<F, T>(
    rustFutureFunc: () -> UInt64,
    pollFunc: (UInt64, @escaping UniffiRustFutureContinuationCallback, UInt64) -> (),
    completeFunc: (UInt64, UnsafeMutablePointer<RustCallStatus>) -> F,
    freeFunc: (UInt64) -> (),
    cancelFunc: @escaping (UInt64) -> (),
    liftFunc: (F) throws -> T,
    errorHandler: ((RustBuffer) throws -> Swift.Error)?
) async throws -> T {
    uniffiEnsureMedialayerInitialized()
    let rustFuture = rustFutureFunc()
    let cancellation = UniffiFutureCancellation(handle: rustFuture, cancelFunc: cancelFunc)
    defer {
        cancellation.finish()
        freeFunc(rustFuture)
    }
    let canCancel = errorHandler != nil
    return try await withTaskCancellationHandler {
        if canCancel { try Task.checkCancellation() }
        var pollResult: Int8
        repeat {
            pollResult = await withUnsafeContinuation {
                pollFunc(
                    rustFuture,
                    { handle, pollResult in
                        uniffiFutureContinuationCallback(handle: handle, pollResult: pollResult)
                    },
                    uniffiContinuationHandleMap.insert(obj: $0)
                )
            }
        } while pollResult != UNIFFI_RUST_FUTURE_POLL_READY
        if canCancel { try Task.checkCancellation() }
        return try liftFunc(makeRustCall(
            { completeFunc(rustFuture, $0) },
            errorHandler: errorHandler
        ))
    } onCancel: {
        if canCancel { cancellation.cancel() }
    }
}
